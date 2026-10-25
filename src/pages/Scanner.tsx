import { useState, useCallback, useRef, useEffect, useLayoutEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save, open as openDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { List } from "react-window";
import {
  ScanSearch, Trash2, CheckSquare, Square,
  AlertTriangle, RotateCcw, ShieldAlert, Undo2, Redo2,
  Download, FileBarChart2, Shield, FolderOpen,
} from "lucide-react";
import { useAppStore, JunkFile, ScanProgress, DeleteResult } from "../store/useAppStore";
import { useOperationStore } from "../store/useOperationStore";
import FileResultRow from "../components/FileResultRow";
import ProgressBar from "../components/ProgressBar";

function fmtSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

export default function Scanner() {
  const {
    scanResults,
    setScanResults,
    scanProgress,
    setScanProgress,
    isScanning,
    setIsScanning,
    selectedFiles,
    selectAllFiles,
    clearSelection,
    setLastScanTime,
  } = useAppStore();

  // Root path to scan — defaults to user home drive (C:\Users\<user>)
  const [scanRoot, setScanRoot] = useState(() => {
    // Best effort: use USERPROFILE env or fall back to C:\Users
    return "C:\\Users";
  });

  const [scanTemp, setScanTemp] = useState(true);
  const [scanDownloads, setScanDownloads] = useState(true);
  const [scanDuplicates, setScanDuplicates] = useState(true);
  const [scanEmptyDirs, setScanEmptyDirs] = useState(true);
  const [scanLarge, setScanLarge] = useState(true);

  // Delete mode: recycle bin (default) vs permanent
  const [permanent, setPermanent] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteResult, setDeleteResult] = useState<DeleteResult | null>(null);
  const [statusMsg, setStatusMsg] = useState("");

  // Phase 4 features
  const [createRestorePoint, setCreateRestorePoint] = useState(false);
  const [restoringPoint, setRestoringPoint] = useState(false);

  // Undo / redo
  const opStore = useOperationStore();

  // Keyboard shortcuts: Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      if (e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        const op = opStore.undo();
        if (op && !op.permanent) {
          setStatusMsg(`Undone: moved ${op.originalPath} back`);
        }
      } else if (e.key === "y" || (e.key === "z" && e.shiftKey)) {
        e.preventDefault();
        opStore.redo();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [opStore]);

  // Accumulate streamed results during scan
  const streamedRef = useRef<JunkFile[]>([]);
  // Keep track of the active scan-progress unlistener so we can clean it up
  // if the component unmounts mid-scan or StrictMode remounts it.
  const unlistenRef = useRef<(() => void) | null>(null);

  // Cleanup: stop listening if the component unmounts (e.g. in dev StrictMode)
  useLayoutEffect(() => {
    return () => {
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, []);

  const browseScanRoot = async () => {
    const dir = await openDialog({ directory: true, multiple: false });
    if (typeof dir === "string") setScanRoot(dir);
  };

  const startScan = useCallback(async () => {
    setIsScanning(true);
    setScanProgress(null);
    setScanResults([]);
    setDeleteResult(null);
    setStatusMsg("");
    streamedRef.current = [];

    // Cancel any previous listener before registering a new one
    unlistenRef.current?.();
    unlistenRef.current = null;

    const unlisten = await listen<ScanProgress>("scan-progress", (ev) => {
      const p = ev.payload;
      setScanProgress(p);

      // Append any batch items to live results immediately
      if (p.batch && p.batch.length > 0) {
        streamedRef.current = [...streamedRef.current, ...p.batch];
        setScanResults([...streamedRef.current]);
      }
    });
    unlistenRef.current = unlisten;

    try {
      // Pass scanRoot so the backend knows where to scan for downloads/large files
      const results = await invoke<JunkFile[]>("scan_for_junk", {
        paths: [scanRoot],
        scanTemp,
        scanDownloads,
        scanDuplicates,
        scanEmptyDirs,
        scanLarge,
      });
      setScanResults(results);
      setLastScanTime(new Date().toLocaleString());
      setStatusMsg(`Found ${results.length} items`);

      // Phase 5 (C3): persist scan index for instant reload next session
      invoke("save_snapshot").catch(() => {/* non-critical */});
    } catch (err) {
      setStatusMsg(`Scan error: ${err}`);
    } finally {
      setIsScanning(false);
      unlisten();
      unlistenRef.current = null;
    }
  }, [
    scanRoot, scanTemp, scanDownloads, scanDuplicates, scanEmptyDirs, scanLarge,
    setIsScanning, setScanProgress, setScanResults, setLastScanTime,
  ]);

  const exportCsv = async () => {
    if (scanResults.length === 0) return;
    const dest = await save({ filters: [{ name: "CSV", extensions: ["csv"] }], defaultPath: "blue-streak-results.csv" });
    if (!dest) return;
    try {
      const count = await invoke<number>("export_scan_csv", { files: scanResults, destPath: dest });
      setStatusMsg(`Exported ${count} rows to CSV`);
    } catch (err) {
      setStatusMsg(`Export error: ${err}`);
    }
  };

  const openReport = async () => {
    if (scanResults.length === 0) return;
    const freedBytes = deleteResult?.freedBytes ?? 0;
    try {
      await invoke("open_html_report", { freedBytes, files: scanResults, periodLabel: "this session" });
    } catch (err) {
      setStatusMsg(`Report error: ${err}`);
    }
  };

  const deleteSelected = async () => {
    const paths = Array.from(selectedFiles);
    if (paths.length === 0) return;

    // Phase 4.6: optionally create a restore point before bulk deletion
    if (createRestorePoint) {
      setRestoringPoint(true);
      setStatusMsg("Creating restore point…");
      try {
        const rp = await invoke<{ success: boolean; message: string }>("create_restore_point", {});
        setStatusMsg(rp.success ? "Restore point created." : `Restore point: ${rp.message}`);
      } catch (_) {
        setStatusMsg("Restore point failed — proceeding anyway.");
      } finally {
        setRestoringPoint(false);
      }
    }

    try {
      const result = await invoke<DeleteResult>("delete_files", { paths, permanent });
      setDeleteResult(result);

      // Remove succeeded paths from results list
      const succeededSet = new Set(result.succeeded);

      // Push each deleted file into undo history
      for (const path of result.succeeded) {
        const file = scanResults.find((f) => f.path === path);
        opStore.push({
          type: "delete",
          originalPath: path,
          sizeBytes: file?.sizeBytes ?? 0,
          permanent,
        });
      }

      const remaining = scanResults.filter((f) => !succeededSet.has(f.path));
      setScanResults(remaining);
      clearSelection();
      setConfirmDelete(false);

      if (result.failed.length === 0) {
        setStatusMsg(`Freed ${fmtSize(result.freedBytes)}`);
      } else {
        setStatusMsg(
          `Freed ${fmtSize(result.freedBytes)} — ${result.failed.length} failed`
        );
      }
    } catch (err) {
      setStatusMsg(`Delete error: ${err}`);
    }
  };

  const totalSelected = selectedFiles.size;
  const selectedSize = scanResults
    .filter((f) => selectedFiles.has(f.path))
    .reduce((a, f) => a + f.sizeBytes, 0);

  // Indeterminate progress: show a pulse bar while scanning
  const progressPct =
    scanProgress && scanProgress.scanned > 0
      ? Math.min(99, (scanProgress.found / Math.max(1, scanProgress.scanned)) * 500)
      : 0;

  const checkboxOptions = [
    { label: "Temp files", value: scanTemp, set: setScanTemp },
    { label: "Old downloads (>90d)", value: scanDownloads, set: setScanDownloads },
    { label: "Duplicate files", value: scanDuplicates, set: setScanDuplicates },
    { label: "Empty folders", value: scanEmptyDirs, set: setScanEmptyDirs },
    { label: "Large files (>500 MB)", value: scanLarge, set: setScanLarge },
  ];

  return (
    <div className="flex h-full overflow-hidden">
      {/* Left panel — options */}
      <div className="w-56 shrink-0 border-r border-surface-600 bg-surface-800 p-5 flex flex-col gap-4">
        {/* Scan root path */}
        <div>
          <h2 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-2">
            Scan Location
          </h2>
          <div className="flex gap-1">
            <input
              value={scanRoot}
              onChange={(e) => setScanRoot(e.target.value)}
              className="flex-1 min-w-0 bg-surface-700 border border-surface-600 rounded px-2 py-1 text-xs text-slate-300 focus:outline-none focus:border-violet-500"
              title="Directory to scan"
            />
            <button
              onClick={browseScanRoot}
              className="p-1.5 rounded bg-surface-700 hover:bg-surface-600 text-slate-400 hover:text-white transition-colors shrink-0"
              title="Browse"
            >
              <FolderOpen size={13} />
            </button>
          </div>
        </div>

        <div>
          <h2 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
            Scan For
          </h2>
          <div className="space-y-2">
            {checkboxOptions.map(({ label, value, set }) => (
              <label
                key={label}
                className="flex items-center gap-2 text-sm text-slate-300 cursor-pointer hover:text-white"
              >
                <input
                  type="checkbox"
                  checked={value}
                  onChange={(e) => set(e.target.checked)}
                  className="w-4 h-4 accent-violet-500"
                />
                {label}
              </label>
            ))}
          </div>
        </div>

        {/* Delete mode toggle */}
        <div className="border-t border-surface-600 pt-4">
          <h2 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
            Delete Mode
          </h2>
          <div className="space-y-2">
            <label className="flex items-center gap-2 text-sm text-slate-300 cursor-pointer hover:text-white">
              <input
                type="radio"
                name="deleteMode"
                checked={!permanent}
                onChange={() => setPermanent(false)}
                className="accent-violet-500"
              />
              <RotateCcw size={13} className="text-green-400" />
              Recycle Bin
            </label>
            <label className="flex items-center gap-2 text-sm text-slate-300 cursor-pointer hover:text-white">
              <input
                type="radio"
                name="deleteMode"
                checked={permanent}
                onChange={() => setPermanent(true)}
                className="accent-violet-500"
              />
              <ShieldAlert size={13} className="text-red-400" />
              Permanent
            </label>
          </div>
          {!permanent && (
            <p className="mt-2 text-[11px] text-green-500/80 leading-snug">
              Files go to Recycle Bin — recoverable
            </p>
          )}
          {permanent && (
            <p className="mt-2 text-[11px] text-red-500/80 leading-snug">
              Files are deleted permanently
            </p>
          )}
        </div>

        {/* Restore point */}
        <div className="border-t border-surface-600 pt-4">
          <h2 className="text-xs font-semibold text-slate-400 uppercase tracking-wider mb-3">
            Safety
          </h2>
          <label className="flex items-center gap-2 text-sm text-slate-300 cursor-pointer hover:text-white">
            <input
              type="checkbox"
              checked={createRestorePoint}
              onChange={(e) => setCreateRestorePoint(e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
            <Shield size={13} className="text-violet-400 shrink-0" />
            Restore point
          </label>
          <p className="mt-1 text-[11px] text-slate-500 leading-snug">
            Create a System Restore Point before deleting
          </p>
        </div>

        <button
          onClick={startScan}
          disabled={isScanning}
          className="mt-auto w-full flex items-center justify-center gap-2 px-4 py-2.5 rounded-lg bg-accent hover:bg-accent-hover text-white text-sm font-medium disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          <ScanSearch size={15} />
          {isScanning ? "Scanning…" : "Scan Now"}
        </button>
      </div>

      {/* Right panel — results */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* Top bar */}
        <div className="px-5 py-3 border-b border-surface-600 bg-surface-800 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {totalSelected > 0 ? (
              <button onClick={clearSelection} className="text-slate-400 hover:text-white">
                <CheckSquare size={15} />
              </button>
            ) : (
              <button onClick={selectAllFiles} className="text-slate-400 hover:text-white">
                <Square size={15} />
              </button>
            )}
            <span className="text-xs text-slate-400">
              {totalSelected > 0
                ? `${totalSelected} selected (${fmtSize(selectedSize)})`
                : `${scanResults.length} items found`}
            </span>
          </div>

          <div className="flex items-center gap-2">
            {/* Undo / Redo */}
            <button
              title="Undo (Ctrl+Z)"
              onClick={() => opStore.undo()}
              disabled={!opStore.canUndo()}
              className="p-1 rounded text-slate-500 hover:text-white disabled:opacity-30 transition-colors"
            >
              <Undo2 size={14} />
            </button>
            <button
              title="Redo (Ctrl+Y)"
              onClick={() => opStore.redo()}
              disabled={!opStore.canRedo()}
              className="p-1 rounded text-slate-500 hover:text-white disabled:opacity-30 transition-colors"
            >
              <Redo2 size={14} />
            </button>

            {/* Phase 4: CSV export & HTML report */}
            {scanResults.length > 0 && (
              <>
                <button
                  title="Export results as CSV"
                  onClick={exportCsv}
                  className="p-1 rounded text-slate-500 hover:text-white transition-colors"
                >
                  <Download size={14} />
                </button>
                <button
                  title="Open HTML report"
                  onClick={openReport}
                  className="p-1 rounded text-slate-500 hover:text-white transition-colors"
                >
                  <FileBarChart2 size={14} />
                </button>
              </>
            )}

            {totalSelected > 0 && (
              <button
                onClick={() => setConfirmDelete(true)}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors ${
                  permanent
                    ? "bg-red-500/20 hover:bg-red-500/30 text-red-400"
                    : "bg-emerald-500/20 hover:bg-emerald-500/30 text-emerald-400"
                }`}
              >
                <Trash2 size={13} />
                {permanent ? "Delete" : "Move to Bin"}
              </button>
            )}
            {statusMsg && (
              <span className="text-xs text-slate-400">{statusMsg}</span>
            )}
          </div>
        </div>

        {/* Progress bar — shown while scanning */}
        {isScanning && (
          <div className="px-5 py-3 border-b border-surface-600 bg-surface-800 space-y-1">
            <ProgressBar
              value={scanProgress?.done ? 100 : progressPct}
              label={
                scanProgress?.currentPath
                  ? `Scanning — ${scanProgress.currentPath}`
                  : "Preparing scan…"
              }
            />
            {scanProgress && (
              <p className="text-[11px] text-slate-500">
                {scanProgress.scanned.toLocaleString()} files scanned
                {scanProgress.found > 0 && ` · ${scanProgress.found} found so far`}
              </p>
            )}
          </div>
        )}

        {/* Partial failure banner */}
        {deleteResult && deleteResult.failed.length > 0 && (
          <div className="px-5 py-3 border-b border-surface-600 bg-red-900/20 flex items-start gap-2">
            <AlertTriangle size={14} className="text-red-400 mt-0.5 shrink-0" />
            <div className="text-xs text-red-300">
              <p className="font-medium mb-1">
                {deleteResult.failed.length} file(s) could not be deleted:
              </p>
              <ul className="space-y-0.5 text-red-400/80">
                {deleteResult.failed.slice(0, 5).map((f) => (
                  <li key={f.path} className="truncate">
                    {f.path} — {f.reason}
                  </li>
                ))}
                {deleteResult.failed.length > 5 && (
                  <li>…and {deleteResult.failed.length - 5} more</li>
                )}
              </ul>
            </div>
            <button
              onClick={() => setDeleteResult(null)}
              className="ml-auto text-red-400/60 hover:text-red-300 text-xs"
            >
              ✕
            </button>
          </div>
        )}

        {/* Results list — virtual scroll via react-window v2 (60 FPS for 100K+ rows).
            react-window v2 List self-sizes via ResizeObserver — no AutoSizer needed. */}
        <div className="flex-1 overflow-hidden">
          {scanResults.length === 0 && !isScanning && (
            <div className="flex flex-col items-center justify-center h-full text-center">
              <ScanSearch size={40} className="text-slate-600 mb-3" />
              <p className="text-slate-500 text-sm">
                No results yet. Run a scan to find junk files.
              </p>
            </div>
          )}
          {scanResults.length > 0 && (
            <List
              className="flex-1"
              style={{ height: "100%", overflowY: "auto", padding: "8px 0" }}
              rowCount={scanResults.length}
              rowHeight={52}
              overscanCount={10}
              rowComponent={FileResultRow}
              rowProps={{ files: scanResults }}
            />
          )}
        </div>

        {/* Footer summary */}
        {scanResults.length > 0 && (
          <div className="px-5 py-3 border-t border-surface-600 bg-surface-800 flex items-center justify-between text-xs text-slate-400">
            <span>
              {scanResults.length} items ·{" "}
              {fmtSize(scanResults.reduce((a, f) => a + f.sizeBytes, 0))} reclaimable
            </span>
            <span>{totalSelected} selected</span>
          </div>
        )}
      </div>

      {/* Delete confirmation dialog */}
      {confirmDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div className="bg-surface-800 border border-surface-600 rounded-xl p-6 max-w-sm mx-4 shadow-2xl space-y-4">
            <div className="flex items-center gap-3">
              {permanent ? (
                <AlertTriangle size={20} className="text-red-400 shrink-0" />
              ) : (
                <RotateCcw size={20} className="text-green-400 shrink-0" />
              )}
              <h2 className="font-semibold text-white text-sm">
                {permanent ? "Permanent Delete" : "Move to Recycle Bin"}
              </h2>
            </div>

            <p className="text-sm text-slate-300">
              {permanent ? (
                <>
                  Permanently delete{" "}
                  <strong className="text-white">{totalSelected} items</strong>{" "}
                  ({fmtSize(selectedSize)})?{" "}
                  <span className="text-red-400">This cannot be undone.</span>
                </>
              ) : (
                <>
                  Move{" "}
                  <strong className="text-white">{totalSelected} items</strong>{" "}
                  ({fmtSize(selectedSize)}) to the Recycle Bin?{" "}
                  <span className="text-green-400">You can restore them later.</span>
                </>
              )}
            </p>

            <div className="flex gap-2 justify-end">
              <button
                onClick={() => setConfirmDelete(false)}
                className="px-4 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-surface-600 transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={deleteSelected}
                className={`px-4 py-2 rounded-lg text-sm font-medium transition-colors ${
                  permanent
                    ? "bg-red-600 hover:bg-red-500 text-white"
                    : "bg-emerald-700 hover:bg-emerald-600 text-white"
                }`}
              >
                {permanent ? "Delete Forever" : "Move to Bin"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
