import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderPlus, X, Save, BatteryLow, Clock, Database, Trash2 } from "lucide-react";
import { useAppStore, AppSettings } from "../store/useAppStore";

interface CacheSummary {
  rootPath: string;
  timestamp: number;
  totalFiles: number;
  durationMs: number;
  ageSeconds: number;
}

function fmtAge(seconds: number): string {
  if (seconds < 60) return `${seconds}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

export default function Settings() {
  const { settings, setSettings, theme, toggleTheme } = useAppStore();
  const [form, setForm] = useState<AppSettings>(settings);
  const [statusMsg, setStatusMsg] = useState("");
  const [cacheSummary, setCacheSummary] = useState<CacheSummary | null>(null);
  const [cacheStatusMsg, setCacheStatusMsg] = useState("");

  useEffect(() => {
    invoke<AppSettings>("get_settings")
      .then((s) => {
        setSettings(s);
        setForm(s);
      })
      .catch(() => {});

    // Try to load cache summary for the temp directory (representative root)
    const tempRoot = "%TEMP%";
    invoke<CacheSummary | null>("get_cache_summary", { rootPath: tempRoot })
      .then((s) => setCacheSummary(s))
      .catch(() => {});
  }, [setSettings]);

  const set = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const addExclusion = async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string" && !form.exclusionPaths.includes(dir)) {
      set("exclusionPaths", [...form.exclusionPaths, dir]);
    }
  };

  const removeExclusion = (p: string) =>
    set("exclusionPaths", form.exclusionPaths.filter((x) => x !== p));

  const handleSave = async () => {
    setStatusMsg("Saving…");
    try {
      await invoke("save_settings", { settings: form });
      setSettings(form);
      setStatusMsg("Settings saved");
      if (form.theme !== theme) toggleTheme();
    } catch (err) {
      setStatusMsg(`Error: ${err}`);
    }
  };

  const handleInvalidateCache = async () => {
    setCacheStatusMsg("Clearing…");
    try {
      await invoke("invalidate_cache", { rootPath: "%TEMP%" });
      setCacheSummary(null);
      setCacheStatusMsg("Cache cleared");
    } catch (err) {
      setCacheStatusMsg(`Error: ${err}`);
    }
  };

  return (
    <div className="flex flex-col h-full overflow-auto">
      <div className="max-w-xl mx-auto w-full py-6 px-4 space-y-6">

        {/* Theme */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-3">
          <h2 className="text-sm font-semibold text-white">Appearance</h2>
          <label className="flex items-center justify-between">
            <span className="text-sm text-slate-300">Dark Mode</span>
            <input
              type="checkbox"
              checked={form.theme === "dark"}
              onChange={(e) => set("theme", e.target.checked ? "dark" : "light")}
              className="w-4 h-4 accent-violet-500"
            />
          </label>
        </section>

        {/* Notifications */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-3">
          <h2 className="text-sm font-semibold text-white">Notifications</h2>
          <label className="flex items-center justify-between">
            <span className="text-sm text-slate-300">Show toast on scan completion</span>
            <input
              type="checkbox"
              checked={form.notificationsEnabled}
              onChange={(e) => set("notificationsEnabled", e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
          </label>
        </section>

        {/* Auto-start */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-3">
          <h2 className="text-sm font-semibold text-white">Startup</h2>
          <label className="flex items-center justify-between">
            <span className="text-sm text-slate-300">Launch CleanSweep on login</span>
            <input
              type="checkbox"
              checked={form.autoStart}
              onChange={(e) => set("autoStart", e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
          </label>
        </section>

        {/* Smart Scheduling — Phase 2.7 */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-4">
          <div>
            <h2 className="text-sm font-semibold text-white">Smart Scheduling</h2>
            <p className="text-xs text-slate-500 mt-0.5">
              Prevent scheduled scans from interrupting active use or draining battery.
            </p>
          </div>

          <label className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              <Clock size={13} className="text-violet-400 shrink-0" />
              <span className="text-sm text-slate-300">Only run when idle</span>
            </div>
            <input
              type="checkbox"
              checked={form.runOnlyWhenIdle}
              onChange={(e) => set("runOnlyWhenIdle", e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
          </label>

          {form.runOnlyWhenIdle && (
            <div className="flex items-center justify-between gap-4 pl-5">
              <span className="text-sm text-slate-400">Idle threshold (minutes)</span>
              <input
                type="number"
                min={1}
                max={60}
                value={form.idleThresholdMinutes}
                onChange={(e) =>
                  set("idleThresholdMinutes", Math.max(1, parseInt(e.target.value, 10) || 5))
                }
                className="w-20 px-2 py-1 rounded-lg bg-surface-700 border border-surface-600 text-sm text-white text-right focus:outline-none focus:ring-1 focus:ring-violet-500"
              />
            </div>
          )}

          <label className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-2">
              <BatteryLow size={13} className="text-amber-400 shrink-0" />
              <span className="text-sm text-slate-300">Require AC power (skip on battery)</span>
            </div>
            <input
              type="checkbox"
              checked={form.requireAcPower}
              onChange={(e) => set("requireAcPower", e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
          </label>
        </section>

        {/* Scan Cache — Phase 2.1 */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Database size={13} className="text-blue-400 shrink-0" />
              <h2 className="text-sm font-semibold text-white">Scan Cache</h2>
            </div>
            <button
              onClick={handleInvalidateCache}
              className="flex items-center gap-1.5 text-xs text-red-400/70 hover:text-red-400 transition-colors"
            >
              <Trash2 size={12} />
              Clear Cache
            </button>
          </div>
          {cacheSummary ? (
            <p className="text-xs text-slate-400">
              Last scan cached{" "}
              <span className="text-slate-200">{fmtAge(cacheSummary.ageSeconds)}</span>
              {cacheSummary.durationMs > 0 && (
                <> · took <span className="text-slate-200">{(cacheSummary.durationMs / 1000).toFixed(1)}s</span></>
              )}
            </p>
          ) : (
            <p className="text-xs text-slate-500">No cached scan found.</p>
          )}
          {cacheStatusMsg && (
            <p className="text-xs text-slate-500">{cacheStatusMsg}</p>
          )}
        </section>

        {/* Scan exclusions */}
        <section className="bg-surface-800 border border-surface-600 rounded-xl p-5 space-y-3">
          <div className="flex items-center justify-between">
            <h2 className="text-sm font-semibold text-white">Scan Exclusions</h2>
            <button
              onClick={addExclusion}
              className="flex items-center gap-1.5 text-xs text-accent-light hover:text-white transition-colors"
            >
              <FolderPlus size={13} />
              Add Folder
            </button>
          </div>
          {form.exclusionPaths.length === 0 ? (
            <p className="text-xs text-slate-500">
              No exclusions. Folders added here will be skipped during scans.
            </p>
          ) : (
            <div className="space-y-1">
              {form.exclusionPaths.map((p) => (
                <div
                  key={p}
                  className="flex items-center gap-2 px-3 py-2 rounded-lg bg-surface-700"
                >
                  <span className="flex-1 text-xs text-slate-300 truncate">{p}</span>
                  <button
                    onClick={() => removeExclusion(p)}
                    className="text-slate-500 hover:text-red-400 transition-colors"
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>

        {/* Save */}
        <div className="flex items-center justify-between">
          <span className="text-xs text-slate-500">{statusMsg}</span>
          <button
            onClick={handleSave}
            className="flex items-center gap-1.5 px-4 py-2 rounded-lg bg-accent hover:bg-accent-hover text-white text-sm font-medium transition-colors"
          >
            <Save size={14} />
            Save Settings
          </button>
        </div>
      </div>
    </div>
  );
}
