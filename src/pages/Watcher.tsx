import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Eye,
  FolderPlus,
  Trash2,
  Play,
  Square,
  X,
} from "lucide-react";
import { useAppStore, WatchEvent } from "../store/useAppStore";

const kindColor: Record<string, string> = {
  created: "text-emerald-400",
  modified: "text-blue-400",
  removed: "text-red-400",
  accessed: "text-slate-400",
  other: "text-slate-500",
};

export default function Watcher() {
  const {
    watcherRunning,
    setWatcherRunning,
    watchedPaths,
    setWatchedPaths,
    watchEvents,
    addWatchEvent,
    clearWatchEvents,
  } = useAppStore();

  const logRef = useRef<HTMLDivElement>(null);

  // Subscribe to watcher events from Tauri
  useEffect(() => {
    let unlisten: (() => void) | null = null;

    listen<WatchEvent>("watcher-event", (ev) => {
      addWatchEvent(ev.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, [addWatchEvent]);

  // Auto-scroll log
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = 0;
    }
  }, [watchEvents.length]);

  const startWatcher = async () => {
    try {
      await invoke("start_watcher", { paths: watchedPaths });
      setWatcherRunning(true);
    } catch (err) {
      console.error("start_watcher error:", err);
    }
  };

  const stopWatcher = async () => {
    try {
      await invoke("stop_watcher");
      setWatcherRunning(false);
    } catch (err) {
      console.error("stop_watcher error:", err);
    }
  };

  const addPath = async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string" && !watchedPaths.includes(dir)) {
      const updated = [...watchedPaths, dir];
      setWatchedPaths(updated);
      if (watcherRunning) {
        // Restart watcher with new paths
        await invoke("stop_watcher");
        await invoke("start_watcher", { paths: updated });
      }
    }
  };

  const removePath = async (p: string) => {
    const updated = watchedPaths.filter((x) => x !== p);
    setWatchedPaths(updated);
    if (watcherRunning) {
      await invoke("stop_watcher");
      if (updated.length > 0) {
        await invoke("start_watcher", { paths: updated });
      } else {
        setWatcherRunning(false);
      }
    }
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="px-6 py-4 border-b border-surface-600 bg-surface-800 flex items-center justify-between shrink-0">
        <div className="flex items-center gap-2">
          <Eye size={16} className="text-violet-400" />
          <h2 className="text-sm font-semibold text-white">Folder Watcher</h2>
          <span
            className={`ml-2 text-xs px-2 py-0.5 rounded-full font-medium ${
              watcherRunning
                ? "bg-emerald-500/20 text-emerald-400"
                : "bg-slate-500/20 text-slate-400"
            }`}
          >
            {watcherRunning ? "Running" : "Stopped"}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={clearWatchEvents}
            className="px-3 py-1.5 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 text-xs transition-colors"
          >
            Clear Log
          </button>
          {watcherRunning ? (
            <button
              onClick={stopWatcher}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-red-600/20 hover:bg-red-600/30 text-red-400 text-xs font-medium transition-colors"
            >
              <Square size={12} />
              Stop
            </button>
          ) : (
            <button
              onClick={startWatcher}
              disabled={watchedPaths.length === 0}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-emerald-600/20 hover:bg-emerald-600/30 text-emerald-400 text-xs font-medium disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
            >
              <Play size={12} />
              Start
            </button>
          )}
        </div>
      </div>

      <div className="flex flex-1 overflow-hidden">
        {/* Left: watched paths */}
        <div className="w-64 shrink-0 border-r border-surface-600 flex flex-col">
          <div className="px-4 py-3 border-b border-surface-600 flex items-center justify-between">
            <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
              Watched Folders
            </span>
            <button
              onClick={addPath}
              className="p-1 rounded hover:bg-surface-600 text-slate-400 hover:text-white transition-colors"
              title="Add folder"
            >
              <FolderPlus size={14} />
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-2 space-y-1">
            {watchedPaths.length === 0 ? (
              <p className="text-xs text-slate-500 text-center mt-6 px-3">
                No folders watched. Click + to add one.
              </p>
            ) : (
              watchedPaths.map((p) => (
                <div
                  key={p}
                  className="flex items-center gap-2 px-3 py-2 rounded-lg bg-surface-700 group"
                >
                  <span className="flex-1 text-xs text-slate-300 truncate" title={p}>
                    {p.split(/[\\/]/).pop()}
                  </span>
                  <button
                    onClick={() => removePath(p)}
                    className="opacity-0 group-hover:opacity-100 text-slate-500 hover:text-red-400 transition-all"
                  >
                    <X size={12} />
                  </button>
                </div>
              ))
            )}
          </div>
        </div>

        {/* Right: event log */}
        <div className="flex-1 flex flex-col overflow-hidden">
          <div className="px-4 py-3 border-b border-surface-600 flex items-center justify-between">
            <span className="text-xs font-semibold text-slate-400 uppercase tracking-wider">
              Live Event Log
            </span>
            <span className="text-xs text-slate-500">{watchEvents.length} events</span>
          </div>
          <div
            ref={logRef}
            className="flex-1 overflow-y-auto font-mono text-xs p-4 space-y-1"
          >
            {watchEvents.length === 0 ? (
              <p className="text-slate-600 text-center mt-8">
                No events yet. Events will appear here when watched folders change.
              </p>
            ) : (
              watchEvents.map((ev, i) => (
                <div key={i} className="flex items-start gap-3 py-0.5">
                  <span className="text-slate-600 shrink-0">{ev.timestamp}</span>
                  <span className={`uppercase text-[10px] font-bold shrink-0 w-12 ${kindColor[ev.kind] ?? "text-slate-400"}`}>
                    {ev.kind}
                  </span>
                  <span className="text-slate-300 break-all">{ev.path}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
