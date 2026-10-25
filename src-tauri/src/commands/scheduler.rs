use std::process::Command;

use crate::models::{ScheduleFrequency, ScheduledTask, TaskAction};

fn action_to_str(action: &TaskAction) -> &'static str {
    match action {
        TaskAction::ScanAndClean => "ScanAndClean",
        TaskAction::RunRules => "RunRules",
        TaskAction::EmptyRecycleBin => "EmptyRecycleBin",
    }
}

fn task_description(action: &TaskAction) -> String {
    match action {
        TaskAction::ScanAndClean => "CleanSweep: Scan and clean junk files".to_string(),
        TaskAction::RunRules => "CleanSweep: Run organizer rules".to_string(),
        TaskAction::EmptyRecycleBin => "CleanSweep: Empty Recycle Bin".to_string(),
    }
}

fn action_run_command(action: &TaskAction) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "cleansweep.exe".to_string());
    let sub = action_to_str(action).to_lowercase();
    format!("\"{}\" --headless --action {}", exe, sub)
}

/// Create a scheduled task via schtasks.exe
#[tauri::command]
pub fn create_scheduled_task(task: ScheduledTask) -> Result<(), String> {
    let task_name = format!("CleanSweep_{}", task.name.replace(' ', "_"));
    let run_cmd = action_run_command(&task.action);
    let description = task_description(&task.action);

    let schedule_type = match task.frequency {
        ScheduleFrequency::Daily => "DAILY",
        ScheduleFrequency::Weekly => "WEEKLY",
        ScheduleFrequency::OnLogin => "ONLOGON",
    };

    let mut args: Vec<String> = vec![
        "/Create".to_string(),
        "/F".to_string(),
        "/TN".to_string(),
        task_name,
        "/TR".to_string(),
        run_cmd,
        "/SC".to_string(),
        schedule_type.to_string(),
        "/RL".to_string(),
        "HIGHEST".to_string(),
        "/IT".to_string(),
    ];

    if let Some(ref time) = task.time {
        if !time.is_empty() && !matches!(task.frequency, ScheduleFrequency::OnLogin) {
            args.push("/ST".to_string());
            args.push(time.clone());
        }
    }

    if let Some(ref day) = task.day_of_week {
        if !day.is_empty() && matches!(task.frequency, ScheduleFrequency::Weekly) {
            args.push("/D".to_string());
            args.push(day.clone());
        }
    }

    let _ = description; // schtasks doesn't have a direct /DESC flag in all versions

    let output = Command::new("schtasks")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run schtasks: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("schtasks error: {}", stderr));
    }

    Ok(())
}

/// List CleanSweep tasks from Windows Task Scheduler
#[tauri::command]
pub fn list_scheduled_tasks() -> Result<Vec<ScheduledTask>, String> {
    let output = Command::new("schtasks")
        .args(["/Query", "/FO", "CSV", "/NH", "/V"])
        .output()
        .map_err(|e| format!("Failed to run schtasks: {}", e))?;

    if !output.status.success() {
        return Err("schtasks query failed".to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tasks: Vec<ScheduledTask> = Vec::new();

    for line in stdout.lines() {
        // Only process CleanSweep tasks
        if !line.contains("CleanSweep_") {
            continue;
        }

        // CSV fields (simplified): TaskName,Next Run Time,Status,...,Last Run Time,...
        let fields: Vec<&str> = line.splitn(10, ',').collect();
        if fields.len() < 4 {
            continue;
        }

        let raw_name = fields[0].trim_matches('"');
        let name = raw_name
            .trim_start_matches(r"\CleanSweep_")
            .replace('_', " ");
        let next_run = fields[1].trim_matches('"').to_string();
        let status = fields[2].trim_matches('"').to_string();
        let last_run = fields.get(4).map(|s| s.trim_matches('"').to_string());

        // Determine action from name
        let action = if name.contains("ScanAndClean") || name.to_lowercase().contains("scan") {
            TaskAction::ScanAndClean
        } else if name.contains("RunRules") || name.to_lowercase().contains("rules") {
            TaskAction::RunRules
        } else {
            TaskAction::EmptyRecycleBin
        };

        tasks.push(ScheduledTask {
            name,
            action,
            frequency: ScheduleFrequency::Daily,
            time: None,
            day_of_week: None,
            last_run,
            next_run: Some(next_run),
            status,
        });
    }

    Ok(tasks)
}

/// Delete a scheduled task by name
#[tauri::command]
pub fn delete_scheduled_task(name: String) -> Result<(), String> {
    let task_name = format!("CleanSweep_{}", name.replace(' ', "_"));

    let output = Command::new("schtasks")
        .args(["/Delete", "/F", "/TN", &task_name])
        .output()
        .map_err(|e| format!("Failed to run schtasks: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("schtasks delete error: {}", stderr));
    }

    Ok(())
}

/// Empty the Windows Recycle Bin via PowerShell
#[tauri::command]
pub fn empty_recycle_bin() -> Result<(), String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
        ])
        .output()
        .map_err(|e| format!("Failed to run powershell: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Clear-RecycleBin error: {}", stderr));
    }

    Ok(())
}
