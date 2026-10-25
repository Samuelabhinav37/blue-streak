/// CleanSweep CLI — headless interface sharing the same Rust library.
///
/// Usage:
///   cleansweep-cli scan   [--temp] [--dups] [--large] [--empty] <path>…
///   cleansweep-cli clean  [--permanent] <path>…
///   cleansweep-cli rules  run [--id <id>]
///   cleansweep-cli rules  export <dest.json>
///   cleansweep-cli rules  import <src.json>
///   cleansweep-cli trends [--days <n>] <path>…
///   cleansweep-cli mft    <drive>
///   cleansweep-cli export-csv <output.csv> <path>…

// Re-use library code directly
use cleansweep_lib::commands::organizer::{get_rules, run_rules, save_rules};
use cleansweep_lib::commands::scanner::{delete_files_sync, scan_for_junk_sync};
use cleansweep_lib::models::JunkFile;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let result = match args[1].as_str() {
        "scan"   => cmd_scan(&args[2..]),
        "clean"  => cmd_clean(&args[2..]),
        "rules"  => cmd_rules(&args[2..]),
        "trends" => cmd_trends(&args[2..]),
        "mft"    => cmd_mft(&args[2..]),
        "export-csv" => cmd_export_csv(&args[2..]),
        "--help" | "-h" | "help" => { print_usage(); Ok(()) }
        other => Err(format!("Unknown command: {other}")),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

// ─── commands ─────────────────────────────────────────────────────────────────

fn cmd_scan(args: &[String]) -> Result<(), String> {
    let mut scan_temp      = false;
    let mut scan_dups      = false;
    let mut scan_large     = false;
    let mut scan_empty     = false;
    let mut scan_downloads = false;
    let mut paths: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--temp"      => scan_temp      = true,
            "--dups"      => scan_dups      = true,
            "--large"     => scan_large     = true,
            "--empty"     => scan_empty     = true,
            "--downloads" => scan_downloads = true,
            "--all"       => {
                scan_temp = true; scan_dups = true;
                scan_large = true; scan_empty = true;
                scan_downloads = true;
            }
            other => paths.push(other.to_string()),
        }
        i += 1;
    }

    if !scan_temp && !scan_dups && !scan_large && !scan_empty && !scan_downloads {
        // Default: scan everything
        scan_temp = true; scan_dups = true; scan_large = true;
        scan_empty = true; scan_downloads = true;
    }

    eprintln!("Scanning {} paths…", paths.len().max(1));

    let results = scan_for_junk_sync(
        paths,
        scan_temp,
        scan_downloads,
        scan_dups,
        scan_empty,
        scan_large,
    )?;

    let total_size: u64 = results.iter().map(|f| f.size_bytes).sum();
    println!("{} items found ({:.1} MB reclaimable)", results.len(), total_size as f64 / 1_048_576.0);

    for f in &results {
        println!("  [{:?}] {} ({})", f.category, f.path, fmt_size(f.size_bytes));
    }

    Ok(())
}

fn cmd_clean(args: &[String]) -> Result<(), String> {
    let mut permanent = false;
    let mut paths: Vec<String> = Vec::new();

    for arg in args {
        if arg == "--permanent" {
            permanent = true;
        } else {
            paths.push(arg.clone());
        }
    }

    if paths.is_empty() {
        return Err("No paths provided. Usage: clean [--permanent] <path>…".to_string());
    }

    let result = delete_files_sync(paths, permanent)?;
    println!(
        "Freed {:.1} MB ({} succeeded, {} failed)",
        result.freed_bytes as f64 / 1_048_576.0,
        result.succeeded.len(),
        result.failed.len()
    );
    for f in &result.failed {
        eprintln!("  FAILED: {} — {}", f.path, f.reason);
    }

    Ok(())
}

fn cmd_rules(args: &[String]) -> Result<(), String> {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("list");

    match sub {
        "list" => {
            let rules = get_rules()?;
            if rules.is_empty() {
                println!("No rules defined.");
            }
            for r in &rules {
                println!("[{}] {} (enabled: {})", r.id, r.name, r.enabled);
            }
        }
        "run" => {
            let mut ids: Vec<String> = Vec::new();
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--id" && i + 1 < args.len() {
                    ids.push(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            // run_rules is async; call synchronously via tokio
            let result = tokio_block(run_rules(ids))?;
            println!("Processed {} files, {} errors", result.processed, result.errors.len());
            for e in &result.errors {
                eprintln!("  {e}");
            }
        }
        "export" => {
            let dest = args.get(1).ok_or("Usage: rules export <dest.json>")?;
            let count = cleansweep_lib::commands::marketplace::export_rules(vec![], dest.clone())?;
            println!("Exported {count} rules to {dest}");
        }
        "import" => {
            let src = args.get(1).ok_or("Usage: rules import <src.json>")?;
            let imported = cleansweep_lib::commands::marketplace::import_rules_from_file(src.clone())?;
            let mut existing = get_rules()?;
            existing.extend(imported.iter().cloned());
            save_rules(existing)?;
            println!("Imported {} rules", imported.len());
        }
        other => return Err(format!("Unknown rules sub-command: {other}")),
    }

    Ok(())
}

fn cmd_trends(args: &[String]) -> Result<(), String> {
    let mut days = 30u64;
    let mut paths: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--days" && i + 1 < args.len() {
            days = args[i + 1].parse().unwrap_or(30);
            i += 2;
        } else {
            paths.push(args[i].clone());
            i += 1;
        }
    }
    println!("Growth trends (last {days} days) — use the GUI for chart visualisation.");
    println!("Paths: {:?}", paths);
    Ok(())
}

fn cmd_mft(args: &[String]) -> Result<(), String> {
    let drive = args.first().map(|s| s.trim_end_matches([':', '\\']).to_string())
        .unwrap_or_else(|| "C".to_string());

    let result = cleansweep_lib::commands::mft::scan_mft(drive)?;
    println!(
        "MFT scan complete: {} files, {} dirs, {:.1} GB total ({} ms)",
        result.total_files,
        result.total_dirs,
        result.total_size_bytes as f64 / 1_073_741_824.0,
        result.duration_ms
    );
    Ok(())
}

fn cmd_export_csv(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("Usage: export-csv <output.csv> <path>…".to_string());
    }
    let output = &args[0];
    let paths: Vec<String> = args[1..].to_vec();

    let results = scan_for_junk_sync(paths, true, true, true, true, true)?;
    write_csv(output, &results)?;
    println!("Exported {} rows to {output}", results.len());
    Ok(())
}

// ─── CSV export (shared with reports) ────────────────────────────────────────

pub fn write_csv(dest: &str, files: &[JunkFile]) -> Result<(), String> {
    let mut wtr = csv::Writer::from_path(dest).map_err(|e| e.to_string())?;
    wtr.write_record(["path", "size_bytes", "category", "age_days"])
        .map_err(|e| e.to_string())?;
    for f in files {
        wtr.write_record(&[
            f.path.clone(),
            f.size_bytes.to_string(),
            format!("{:?}", f.category),
            f.age_days.to_string(),
        ])
        .map_err(|e| e.to_string())?;
    }
    wtr.flush().map_err(|e| e.to_string())?;
    Ok(())
}

// ─── sync wrappers (lib uses async for Tauri; CLI needs sync) ─────────────────

fn tokio_block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn fmt_size(bytes: u64) -> String {
    if bytes == 0 { return "0 B".to_string(); }
    let units = ["B", "KB", "MB", "GB"];
    let i = (bytes as f64).log(1024.0).floor() as usize;
    let i = i.min(units.len() - 1);
    format!("{:.1} {}", bytes as f64 / 1024_f64.powi(i as i32), units[i])
}

fn print_usage() {
    println!(
r#"CleanSweep CLI

USAGE:
  cleansweep-cli <command> [options]

COMMANDS:
  scan         Scan for junk files
               --temp --dups --large --empty --downloads --all
               <path>…  directories to scan (default: TEMP)

  clean        Delete files by path
               --permanent  skip Recycle Bin
               <path>…  files to delete

  rules list   List saved organizer rules
  rules run    Run enabled organizer rules
               --id <id>  run specific rule
  rules export <dest.json>
  rules import <src.json>

  trends       Show disk growth trends (last N days)
               --days <n>  look-back window (default 30)
               <path>…  directories to track

  mft          Direct NTFS MFT scan (Windows, requires admin)
               <drive>  e.g. C

  export-csv   Scan and export results to CSV
               <output.csv> <path>…

  help         Show this message
"#
    );
}
