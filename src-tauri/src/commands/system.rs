/// System state queries: idle time, battery/AC power, drive type.
/// Used by the scheduler to run tasks only when the system is idle and on AC.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemState {
    pub idle_seconds: u64,
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
}

// ─── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
mod win {
    pub fn idle_seconds() -> u64 {
        unsafe {
            let mut info: winapi::um::winuser::LASTINPUTINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<winapi::um::winuser::LASTINPUTINFO>() as u32;
            if winapi::um::winuser::GetLastInputInfo(&mut info) != 0 {
                let tick = winapi::um::sysinfoapi::GetTickCount();
                let elapsed = tick.wrapping_sub(info.dwTime);
                return (elapsed / 1000) as u64;
            }
            0
        }
    }

    pub fn on_battery() -> (bool, Option<u8>) {
        unsafe {
            let mut status: winapi::um::winbase::SYSTEM_POWER_STATUS = std::mem::zeroed();
            if winapi::um::winbase::GetSystemPowerStatus(&mut status) != 0 {
                // ACLineStatus: 0 = battery, 1 = AC, 255 = unknown
                let on_batt = status.ACLineStatus == 0;
                // BatteryLifePercent: 0–100, 255 = unknown
                let pct = if status.BatteryLifePercent <= 100 {
                    Some(status.BatteryLifePercent)
                } else {
                    None
                };
                return (on_batt, pct);
            }
            (false, None)
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod win {
    pub fn idle_seconds() -> u64 { 0 }
    pub fn on_battery() -> (bool, Option<u8>) { (false, None) }
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Query current idle time and power state.
#[tauri::command]
pub fn get_system_state() -> SystemState {
    let idle = win::idle_seconds();
    let (on_battery, battery_percent) = win::on_battery();
    SystemState { idle_seconds: idle, on_battery, battery_percent }
}

/// Returns true if the system meets the conditions for running a background task:
/// - Idle for at least `min_idle_seconds`
/// - Not on battery (if `require_ac` is true)
#[tauri::command]
pub fn should_run_task(min_idle_seconds: u64, require_ac: bool) -> bool {
    let state = get_system_state();
    let idle_ok = state.idle_seconds >= min_idle_seconds;
    let power_ok = !require_ac || !state.on_battery;
    idle_ok && power_ok
}

// ─── Phase 3.6: Volume Shadow Copy — System Restore Point ────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePointResult {
    pub success: bool,
    pub message: String,
}

/// Create a Windows System Restore Point before a bulk cleanup operation.
///
/// Uses PowerShell `Checkpoint-Computer` which calls the VSS `SRSetRestorePoint`
/// API. Requires System Protection to be enabled on the target drive (on by
/// default for C:\).
///
/// The function is intentionally synchronous so callers can await it before
/// beginning a destructive scan.
#[tauri::command]
pub fn create_restore_point(description: Option<String>) -> RestorePointResult {
    let desc = description
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "CleanSweep pre-cleanup checkpoint".to_string());

    #[cfg(target_os = "windows")]
    {
        // Checkpoint-Computer requires admin; if UAC is not elevated it returns
        // a non-zero exit code — we surface the stderr message to the frontend.
        let script = format!(
            "Checkpoint-Computer -Description '{}' -RestorePointType MODIFY_SETTINGS",
            desc.replace('\'', "\\'")
        );

        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
        {
            Ok(out) if out.status.success() => RestorePointResult {
                success: true,
                message: format!("Restore point \"{}\" created successfully.", desc),
            },
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let detail = if !stderr.is_empty() { stderr } else { stdout };
                RestorePointResult {
                    success: false,
                    message: format!(
                        "Failed to create restore point (is System Protection enabled?): {}",
                        detail
                    ),
                }
            }
            Err(e) => RestorePointResult {
                success: false,
                message: format!("Could not launch PowerShell: {}", e),
            },
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = desc;
        RestorePointResult {
            success: false,
            message: "System Restore Points are only available on Windows.".to_string(),
        }
    }
}
