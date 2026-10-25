import { useEffect, useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  HardDrive, Trash2, Clock, CalendarClock, ScanSearch, Zap, FolderSync,
  TrendingUp, TrendingDown, Minus,
} from "lucide-react";
import { useAppStore } from "../store/useAppStore";
import DiskUsageRing from "../components/DiskUsageRing";

function fmtSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

interface GrowthTrend {
  path: string;
  points: { capturedAt: number; sizeBytes: number; fileCount: number }[];
  deltaBytes: number;
  spanDays: number;
}

export default function Dashboard() {
  const {
    diskUsage,
    setDiskUsage,
    scanResults,
    lastScanTime,
    scheduledTasks,
    setActivePage,
  } = useAppStore();

  const [trends, setTrends] = useState<GrowthTrend[]>([]);

  const loadDisk = useCallback(async () => {
    try {
      const usage = await invoke<{ totalBytes: number; freeBytes: number; usedBytes: number }>(
        "get_disk_usage",
        { drive: "C:" }
      );
      setDiskUsage(usage);
    } catch (_) {
      // ignore
    }
  }, [setDiskUsage]);

  const loadTrends = useCallback(async () => {
    try {
      const t = await invoke<GrowthTrend[]>("get_growth_trends", {
        paths: ["C:\\Users"],
        days: 30,
      });
      setTrends(t);
    } catch (_) {
      // growth trends not yet captured — ignore silently
    }
  }, []);

  useEffect(() => {
    loadDisk();
    loadTrends();
  }, [loadDisk, loadTrends]);

  const junkSize = scanResults.reduce((a, f) => a + f.sizeBytes, 0);

  const stats = [
    {
      label: "Disk Used",
      value: diskUsage ? fmtSize(diskUsage.usedBytes) : "—",
      sub: diskUsage ? `of ${fmtSize(diskUsage.totalBytes)}` : "loading…",
      icon: HardDrive,
      color: "text-violet-400",
    },
    {
      label: "Junk Found",
      value: scanResults.length > 0 ? `${scanResults.length} files` : "Not scanned",
      sub: scanResults.length > 0 ? fmtSize(junkSize) + " to free" : "Run a scan",
      icon: Trash2,
      color: "text-red-400",
    },
    {
      label: "Last Scan",
      value: lastScanTime ?? "Never",
      sub: "file scanner",
      icon: Clock,
      color: "text-blue-400",
    },
    {
      label: "Scheduled Tasks",
      value: `${scheduledTasks.length}`,
      sub: "active tasks",
      icon: CalendarClock,
      color: "text-emerald-400",
    },
  ];

  return (
    <div className="p-6 space-y-6">
      {/* Stat cards */}
      <div className="grid grid-cols-2 xl:grid-cols-4 gap-4">
        {stats.map(({ label, value, sub, icon: Icon, color }) => (
          <div
            key={label}
            className="bg-surface-800 border border-surface-600 rounded-xl p-4 space-y-2"
          >
            <div className="flex items-center justify-between">
              <p className="text-xs text-slate-400 font-medium">{label}</p>
              <Icon size={16} className={color} />
            </div>
            <p className="text-lg font-semibold text-white">{value}</p>
            <p className="text-xs text-slate-500">{sub}</p>
          </div>
        ))}
      </div>

      {/* Disk ring + Quick actions */}
      <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
        {/* Disk ring */}
        <div className="bg-surface-800 border border-surface-600 rounded-xl p-5">
          <h2 className="text-sm font-semibold text-white mb-4">Disk Usage — C:</h2>
          <div className="flex items-center gap-8">
            <DiskUsageRing
              usedBytes={diskUsage?.usedBytes ?? 0}
              totalBytes={diskUsage?.totalBytes ?? 1}
              size={130}
            />
            <div className="space-y-2 text-sm">
              <div className="flex items-center gap-2">
                <span className="w-3 h-3 rounded-full bg-accent inline-block" />
                <span className="text-slate-300">Used: {diskUsage ? fmtSize(diskUsage.usedBytes) : "—"}</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="w-3 h-3 rounded-full bg-surface-600 inline-block" />
                <span className="text-slate-300">Free: {diskUsage ? fmtSize(diskUsage.freeBytes) : "—"}</span>
              </div>
              <div className="flex items-center gap-2">
                <span className="w-3 h-3 rounded-full bg-slate-600 inline-block" />
                <span className="text-slate-300">Total: {diskUsage ? fmtSize(diskUsage.totalBytes) : "—"}</span>
              </div>
            </div>
          </div>
        </div>

        {/* Quick actions */}
        <div className="bg-surface-800 border border-surface-600 rounded-xl p-5">
          <h2 className="text-sm font-semibold text-white mb-4">Quick Actions</h2>
          <div className="space-y-2">
            <button
              onClick={() => setActivePage("scanner")}
              className="w-full flex items-center gap-3 px-4 py-3 rounded-lg bg-surface-700 hover:bg-accent/20 hover:border-accent/40 border border-surface-500 text-sm text-slate-300 hover:text-white transition-colors"
            >
              <ScanSearch size={16} className="text-violet-400" />
              Scan for Junk Files
            </button>
            <button
              onClick={() => setActivePage("scanner")}
              className="w-full flex items-center gap-3 px-4 py-3 rounded-lg bg-surface-700 hover:bg-red-500/10 hover:border-red-500/30 border border-surface-500 text-sm text-slate-300 hover:text-white transition-colors"
            >
              <Zap size={16} className="text-red-400" />
              Clean Selected Junk
            </button>
            <button
              onClick={() => setActivePage("organizer")}
              className="w-full flex items-center gap-3 px-4 py-3 rounded-lg bg-surface-700 hover:bg-blue-500/10 hover:border-blue-500/30 border border-surface-500 text-sm text-slate-300 hover:text-white transition-colors"
            >
              <FolderSync size={16} className="text-blue-400" />
              Run Organizer Rules
            </button>
          </div>
        </div>
      </div>

      {/* Disk growth trends — Phase 4.2 */}
      {trends.length > 0 && (
        <div className="bg-surface-800 border border-surface-600 rounded-xl p-5">
          <h2 className="text-sm font-semibold text-white mb-3">
            Disk Growth Trends <span className="text-xs text-slate-500 font-normal ml-1">(last 30 days)</span>
          </h2>
          <div className="space-y-2">
            {trends.slice(0, 5).map((t) => {
              const Icon = t.deltaBytes > 0 ? TrendingUp : t.deltaBytes < 0 ? TrendingDown : Minus;
              const color = t.deltaBytes > 500_000_000 ? "text-red-400" : t.deltaBytes > 0 ? "text-amber-400" : "text-emerald-400";
              return (
                <div key={t.path} className="flex items-center justify-between py-1 border-b border-surface-600 last:border-0">
                  <div className="flex items-center gap-2 min-w-0">
                    <Icon size={13} className={color} />
                    <span className="text-xs text-slate-400 truncate max-w-xs" title={t.path}>
                      {t.path.split(/[\\/]/).slice(-2).join("/")}
                    </span>
                  </div>
                  <span className={`text-xs font-medium shrink-0 ml-2 ${color}`}>
                    {t.deltaBytes > 0 ? "+" : ""}{fmtSize(Math.abs(t.deltaBytes))}
                  </span>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Recent junk summary */}
      {scanResults.length > 0 && (
        <div className="bg-surface-800 border border-surface-600 rounded-xl p-5">
          <h2 className="text-sm font-semibold text-white mb-3">Recent Scan Results</h2>
          <div className="space-y-1 max-h-48 overflow-y-auto">
            {scanResults.slice(0, 10).map((f) => (
              <div
                key={f.path}
                className="flex items-center justify-between text-xs py-1 border-b border-surface-600 last:border-0"
              >
                <span className="text-slate-400 truncate max-w-xs">{f.path}</span>
                <span className="text-slate-500 shrink-0 ml-2">{fmtSize(f.sizeBytes)}</span>
              </div>
            ))}
            {scanResults.length > 10 && (
              <p className="text-xs text-slate-500 pt-1">
                …and {scanResults.length - 10} more
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
