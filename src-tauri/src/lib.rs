pub mod commands;
pub mod models;

use commands::audio_hash::scan_audio_duplicates;
use commands::cache::{get_cache_summary, invalidate_cache, new_shared_cache};
use commands::growth::{capture_growth_snapshot, get_growth_trends, prune_growth_snapshots};
use commands::image_hash::scan_similar_images;
use commands::lsh::scan_near_duplicates;
use commands::marketplace::{export_rules, import_rules_from_file, import_rules_from_url, save_merged_rules};
use commands::mft::scan_mft;
use commands::network::{classify_path, scan_network_path};
use commands::organizer::{get_rules, run_rules, save_rules};
use commands::reports::{export_scan_csv, notify_cleanup_complete, open_html_report};
use commands::scanner::{delete_files, get_disk_usage, scan_for_junk};
use commands::treemap::{compute_treemap, compute_treemap_cached, get_scan_index_summary};
use commands::watcher::{
    get_watcher_events, is_watcher_running, new_watcher_state, start_watcher, stop_watcher,
};
use commands::scheduler::{
    create_scheduled_task, delete_scheduled_task, empty_recycle_bin, list_scheduled_tasks,
};
use commands::settings::{get_settings, save_settings};
use commands::snapshot::{delete_snapshot, load_snapshot, save_snapshot};
use commands::system::{create_restore_point, get_system_state, should_run_task};
use commands::usn::{apply_usn_delta, usn_get_changes, usn_snapshot};
use models::{new_shared_scan_index};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let watcher_state = new_watcher_state();
    let cache = new_shared_cache();
    let scan_index = new_shared_scan_index();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .manage(watcher_state)
        .manage(cache)
        .manage(scan_index)
        .invoke_handler(tauri::generate_handler![
            // scanner
            scan_for_junk,
            delete_files,
            get_disk_usage,
            // cache
            get_cache_summary,
            invalidate_cache,
            // image hashing
            scan_similar_images,
            // treemap (Phase 5: cached variant queries SharedScanIndex)
            compute_treemap,
            compute_treemap_cached,
            get_scan_index_summary,
            // organizer
            get_rules,
            save_rules,
            run_rules,
            // watcher
            start_watcher,
            stop_watcher,
            get_watcher_events,
            is_watcher_running,
            // scheduler
            list_scheduled_tasks,
            create_scheduled_task,
            delete_scheduled_task,
            empty_recycle_bin,
            // settings
            get_settings,
            save_settings,
            // system
            get_system_state,
            should_run_task,
            create_restore_point,
            // usn journal
            usn_snapshot,
            usn_get_changes,
            apply_usn_delta,
            // Phase 5: snapshot persistence
            save_snapshot,
            load_snapshot,
            delete_snapshot,
            // mft direct scan (Phase 3.4)
            scan_mft,
            // lsh near-duplicate detection (Phase 3.5)
            scan_near_duplicates,
            // Phase 4: audio duplicates
            scan_audio_duplicates,
            // Phase 4: disk growth trends
            capture_growth_snapshot,
            get_growth_trends,
            prune_growth_snapshots,
            // Phase 4: rule marketplace
            export_rules,
            import_rules_from_file,
            import_rules_from_url,
            save_merged_rules,
            // Phase 4: network / WSL2
            classify_path,
            scan_network_path,
            // Phase 4: reports & notifications
            export_scan_csv,
            open_html_report,
            notify_cleanup_complete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
