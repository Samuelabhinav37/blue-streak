use std::sync::{Arc, Mutex};

use chrono::Local;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, State};

use crate::models::WatchEvent;

pub struct WatcherState {
    pub watcher: Option<RecommendedWatcher>,
    pub events: Vec<WatchEvent>,
    pub running: bool,
}

pub type SharedWatcherState = Arc<Mutex<WatcherState>>;

pub fn new_watcher_state() -> SharedWatcherState {
    Arc::new(Mutex::new(WatcherState {
        watcher: None,
        events: Vec::new(),
        running: false,
    }))
}

#[tauri::command]
pub fn start_watcher(
    app: AppHandle,
    paths: Vec<String>,
    state: State<'_, SharedWatcherState>,
) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;

    if guard.running {
        return Ok(());
    }

    let app_clone = app.clone();
    let state_arc = Arc::clone(&*state);

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            let kind_str = match &event.kind {
                EventKind::Create(_) => "created",
                EventKind::Modify(_) => "modified",
                EventKind::Remove(_) => "removed",
                EventKind::Access(_) => "accessed",
                _ => "other",
            };

            for path in &event.paths {
                let we = WatchEvent {
                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                    path: path.to_string_lossy().to_string(),
                    kind: kind_str.to_string(),
                };

                if let Ok(mut g) = state_arc.lock() {
                    g.events.push(we.clone());
                    if g.events.len() > 500 {
                        g.events.remove(0);
                    }
                }

                let _ = app_clone.emit("watcher-event", &we);
            }
        }
    })
    .map_err(|e| e.to_string())?;

    for p in &paths {
        let path = std::path::Path::new(p);
        if path.exists() {
            watcher
                .watch(path, RecursiveMode::NonRecursive)
                .map_err(|e| e.to_string())?;
        }
    }

    guard.watcher = Some(watcher);
    guard.running = true;

    Ok(())
}

#[tauri::command]
pub fn stop_watcher(state: State<'_, SharedWatcherState>) -> Result<(), String> {
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.watcher = None;
    guard.running = false;
    Ok(())
}

#[tauri::command]
pub fn get_watcher_events(
    state: State<'_, SharedWatcherState>,
) -> Result<Vec<WatchEvent>, String> {
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok(guard.events.clone())
}

#[tauri::command]
pub fn is_watcher_running(state: State<'_, SharedWatcherState>) -> Result<bool, String> {
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok(guard.running)
}
