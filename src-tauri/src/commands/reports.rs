/// Notifications and reports — Phase 4.6
///
/// Features:
///   1. Windows toast notification via PowerShell (wraps tauri-plugin-notification
///      with a richer message format)
///   2. CSV export of scan results
///   3. HTML weekly summary report — "You freed X GB this week"
///      Opens in the default browser via the `open` crate.

use serde::{Deserialize, Serialize};

use crate::models::JunkFile;

// ─── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportEntry {
    pub path: String,
    pub size_bytes: u64,
    pub category: String,
    pub age_days: i64,
}

// ─── CSV export ───────────────────────────────────────────────────────────────

/// Export `files` as a CSV to `dest_path`.
/// Columns: path, size_bytes, category, age_days
#[tauri::command]
pub fn export_scan_csv(
    files: Vec<JunkFile>,
    dest_path: String,
) -> Result<u32, String> {
    let mut wtr = csv::Writer::from_path(&dest_path).map_err(|e| e.to_string())?;
    wtr.write_record(["path", "size_bytes", "category", "age_days"])
        .map_err(|e| e.to_string())?;

    for f in &files {
        wtr.write_record(&[
            f.path.clone(),
            f.size_bytes.to_string(),
            format!("{:?}", f.category),
            f.age_days.to_string(),
        ])
        .map_err(|e| e.to_string())?;
    }
    wtr.flush().map_err(|e| e.to_string())?;

    Ok(files.len() as u32)
}

// ─── HTML weekly report ───────────────────────────────────────────────────────

/// Generate an HTML summary report and open it in the default browser.
///
/// `freed_bytes`: total bytes freed during the period.
/// `files`: list of items that were cleaned.
/// `period_label`: human-readable period, e.g. "this week" or "last 30 days".
#[tauri::command]
pub fn open_html_report(
    freed_bytes: u64,
    files: Vec<JunkFile>,
    period_label: Option<String>,
) -> Result<String, String> {
    let period = period_label.unwrap_or_else(|| "this session".to_string());
    let html = build_report_html(freed_bytes, &files, &period);

    // Write to a temp file
    let tmp_path = std::env::temp_dir().join("cleansweep_report.html");
    std::fs::write(&tmp_path, &html).map_err(|e| e.to_string())?;

    // Open in default browser
    open::that(&tmp_path).map_err(|e| e.to_string())?;

    Ok(tmp_path.to_string_lossy().into_owned())
}

fn fmt_size(bytes: u64) -> String {
    if bytes == 0 { return "0 B".to_string(); }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let i = (bytes as f64).log(1024.0).floor() as usize;
    let i = i.min(units.len() - 1);
    format!("{:.1} {}", bytes as f64 / 1024_f64.powi(i as i32), units[i])
}

fn build_report_html(freed_bytes: u64, files: &[JunkFile], period: &str) -> String {
    // Count by category
    use std::collections::HashMap;
    let mut by_cat: HashMap<String, (u64, u64)> = HashMap::new(); // cat -> (count, bytes)
    for f in files {
        let cat = format!("{:?}", f.category);
        let e = by_cat.entry(cat).or_default();
        e.0 += 1;
        e.1 += f.size_bytes;
    }

    let mut rows = by_cat
        .iter()
        .map(|(cat, (count, size))| {
            format!(
                "<tr><td>{cat}</td><td style='text-align:right'>{count}</td><td style='text-align:right'>{}</td></tr>",
                fmt_size(*size)
            )
        })
        .collect::<Vec<_>>();
    rows.sort(); // deterministic order

    let file_rows: String = files
        .iter()
        .take(200) // cap at 200 rows for readability
        .map(|f| {
            format!(
                "<tr><td style='max-width:400px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap'>{}</td><td style='text-align:right'>{}</td><td>{:?}</td><td style='text-align:right'>{} d</td></tr>",
                html_escape(&f.path),
                fmt_size(f.size_bytes),
                f.category,
                f.age_days,
            )
        })
        .collect();

    let truncation_note = if files.len() > 200 {
        format!("<p style='color:#888;font-size:12px'>Showing first 200 of {} items.</p>", files.len())
    } else {
        String::new()
    };

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>CleanSweep Report</title>
<style>
  body {{ font-family: -apple-system, 'Segoe UI', sans-serif; background:#f7f8fa; color:#1f2328; margin:0; padding:0; }}
  .wrap {{ max-width:760px; margin:32px auto; padding:0 16px; }}
  h1 {{ font-size:22px; font-weight:700; margin-bottom:4px; }}
  .sub {{ color:#57606a; font-size:13px; margin-bottom:24px; }}
  .hero {{ background:#3b82d4; color:#fff; border-radius:12px; padding:24px 32px; margin-bottom:24px; }}
  .hero-val {{ font-size:40px; font-weight:800; letter-spacing:-1px; }}
  .hero-lbl {{ font-size:14px; opacity:.85; }}
  table {{ width:100%; border-collapse:collapse; font-size:13px; margin-bottom:24px; }}
  th {{ background:#e5e7eb; padding:8px 10px; text-align:left; font-weight:600; }}
  td {{ padding:6px 10px; border-bottom:1px solid #e5e7eb; }}
  tr:last-child td {{ border-bottom:none; }}
  .card {{ background:#fff; border:1px solid #e5e7eb; border-radius:10px; overflow:hidden; margin-bottom:24px; }}
  footer {{ text-align:center; font-size:11px; color:#57606a; border-top:1px solid #e5e7eb; padding:12px 0; margin-top:32px; }}
</style>
</head>
<body>
<div class="wrap">
  <h1>CleanSweep Report</h1>
  <p class="sub">Generated {now} &middot; Period: {period}</p>

  <div class="hero">
    <div class="hero-val">{freed_str}</div>
    <div class="hero-lbl">freed {period}</div>
  </div>

  <div class="card">
    <table>
      <thead><tr><th>Category</th><th style="text-align:right">Files</th><th style="text-align:right">Size</th></tr></thead>
      <tbody>{category_rows}</tbody>
    </table>
  </div>

  <h2 style="font-size:15px;margin-bottom:8px">File Details</h2>
  {truncation_note}
  <div class="card">
    <table>
      <thead><tr><th>Path</th><th style="text-align:right">Size</th><th>Category</th><th style="text-align:right">Age</th></tr></thead>
      <tbody>{file_rows}</tbody>
    </table>
  </div>

  <footer>Made with CleanSweep</footer>
</div>
</body>
</html>"#,
        now = now,
        period = period,
        freed_str = fmt_size(freed_bytes),
        category_rows = rows.join("\n"),
        file_rows = file_rows,
        truncation_note = truncation_note,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}

// ─── Toast notification ───────────────────────────────────────────────────────

/// Send a Windows toast notification after a cleanup operation.
/// Wraps PowerShell `New-BurntToastNotification` / fallback `msg`.
#[tauri::command]
pub fn notify_cleanup_complete(
    freed_bytes: u64,
    item_count: u64,
    title: Option<String>,
) -> Result<(), String> {
    let title_str = title.unwrap_or_else(|| "CleanSweep".to_string());
    let body = format!(
        "Freed {} across {} item{}.",
        fmt_size(freed_bytes),
        item_count,
        if item_count == 1 { "" } else { "s" }
    );

    #[cfg(target_os = "windows")]
    {
        // Try BurntToast (common PowerShell module) first; fall back to balloon tip
        let script = format!(
            r#"
$title = '{title}'
$body  = '{body}'
if (Get-Command New-BurntToastNotification -ErrorAction SilentlyContinue) {{
    New-BurntToastNotification -Text $title, $body
}} else {{
    Add-Type -AssemblyName System.Windows.Forms
    $n = New-Object System.Windows.Forms.NotifyIcon
    $n.Icon = [System.Drawing.SystemIcons]::Information
    $n.BalloonTipTitle = $title
    $n.BalloonTipText  = $body
    $n.Visible = $true
    $n.ShowBalloonTip(4000)
    Start-Sleep -Seconds 4
    $n.Dispose()
}}
"#,
            title = title_str.replace('\'', "\\'"),
            body  = body.replace('\'', "\\'"),
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
            .spawn();
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (freed_bytes, item_count, title_str, body);
        Ok(())
    }
}
