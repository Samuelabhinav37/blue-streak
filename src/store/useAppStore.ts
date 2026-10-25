import { create } from "zustand";

// ── Types ──────────────────────────────────────────────────────────────────────

export type JunkCategory = "Temp" | "Download" | "Duplicate" | "EmptyDir" | "Large" | "PossibleDuplicate";

export interface JunkFile {
  path: string;
  sizeBytes: number;
  category: JunkCategory;
  ageDays: number;
}

export interface ScanProgress {
  scanned: number;
  found: number;
  currentPath: string;
  done: boolean;
  batch: JunkFile[];
}

export interface DeleteResult {
  freedBytes: number;
  succeeded: string[];
  failed: { path: string; reason: string }[];
}

export type OrganizerAction = "Move" | "Copy" | "Archive";

export interface RuleConditions {
  extensions: string[];
  minSizeMb: number | null;
  maxSizeMb: number | null;
  olderThanDays: number | null;
  namePatternRegex: string | null;
}

export interface OrganizerRule {
  id: string;
  name: string;
  sourceFolder: string;
  conditions: RuleConditions;
  destination: string;
  action: OrganizerAction;
  enabled: boolean;
}

export interface WatchEvent {
  timestamp: string;
  path: string;
  kind: string;
}

export type TaskAction = "ScanAndClean" | "RunRules" | "EmptyRecycleBin";
export type ScheduleFrequency = "Daily" | "Weekly" | "OnLogin";

export interface ScheduledTask {
  name: string;
  action: TaskAction;
  frequency: ScheduleFrequency;
  time: string | null;
  dayOfWeek: string | null;
  lastRun: string | null;
  nextRun: string | null;
  status: string;
}

export interface AppSettings {
  theme: "dark" | "light";
  exclusionPaths: string[];
  notificationsEnabled: boolean;
  autoStart: boolean;
  /** Phase 2.7 — only run scheduled tasks when the system is idle */
  runOnlyWhenIdle: boolean;
  /** Minimum idle time in minutes before a scheduled task may run */
  idleThresholdMinutes: number;
  /** Phase 2.7 — skip scheduled tasks while on battery power */
  requireAcPower: boolean;
}

export interface DiskUsage {
  totalBytes: number;
  freeBytes: number;
  usedBytes: number;
}

// ── Store ──────────────────────────────────────────────────────────────────────

interface AppState {
  // Navigation
  activePage: string;
  setActivePage: (page: string) => void;

  // Scanner
  scanResults: JunkFile[];
  setScanResults: (r: JunkFile[]) => void;
  scanProgress: ScanProgress | null;
  setScanProgress: (p: ScanProgress | null) => void;
  isScanning: boolean;
  setIsScanning: (v: boolean) => void;
  selectedFiles: Set<string>;
  toggleFileSelected: (path: string) => void;
  selectAllFiles: () => void;
  clearSelection: () => void;
  lastScanTime: string | null;
  setLastScanTime: (t: string) => void;

  // Organizer
  rules: OrganizerRule[];
  setRules: (r: OrganizerRule[]) => void;

  // Watcher
  watcherRunning: boolean;
  setWatcherRunning: (v: boolean) => void;
  watchedPaths: string[];
  setWatchedPaths: (p: string[]) => void;
  watchEvents: WatchEvent[];
  addWatchEvent: (e: WatchEvent) => void;
  clearWatchEvents: () => void;

  // Scheduler
  scheduledTasks: ScheduledTask[];
  setScheduledTasks: (t: ScheduledTask[]) => void;

  // Settings
  settings: AppSettings;
  setSettings: (s: AppSettings) => void;

  // Disk usage
  diskUsage: DiskUsage | null;
  setDiskUsage: (d: DiskUsage) => void;

  // Theme
  theme: "dark" | "light";
  toggleTheme: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  activePage: "dashboard",
  setActivePage: (page) => set({ activePage: page }),

  scanResults: [],
  setScanResults: (scanResults) => set({ scanResults }),
  scanProgress: null,
  setScanProgress: (scanProgress) => set({ scanProgress }),
  isScanning: false,
  setIsScanning: (isScanning) => set({ isScanning }),
  selectedFiles: new Set(),
  toggleFileSelected: (path) => {
    const next = new Set(get().selectedFiles);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    set({ selectedFiles: next });
  },
  selectAllFiles: () => {
    const all = new Set(get().scanResults.map((f) => f.path));
    set({ selectedFiles: all });
  },
  clearSelection: () => set({ selectedFiles: new Set() }),
  lastScanTime: null,
  setLastScanTime: (lastScanTime) => set({ lastScanTime }),

  rules: [],
  setRules: (rules) => set({ rules }),

  watcherRunning: false,
  setWatcherRunning: (watcherRunning) => set({ watcherRunning }),
  watchedPaths: [],
  setWatchedPaths: (watchedPaths) => set({ watchedPaths }),
  watchEvents: [],
  addWatchEvent: (e) => {
    const next = [e, ...get().watchEvents].slice(0, 200);
    set({ watchEvents: next });
  },
  clearWatchEvents: () => set({ watchEvents: [] }),

  scheduledTasks: [],
  setScheduledTasks: (scheduledTasks) => set({ scheduledTasks }),

  settings: {
    theme: "dark",
    exclusionPaths: [],
    notificationsEnabled: true,
    autoStart: false,
    runOnlyWhenIdle: false,
    idleThresholdMinutes: 5,
    requireAcPower: false,
  },
  setSettings: (settings) => set({ settings }),

  diskUsage: null,
  setDiskUsage: (diskUsage) => set({ diskUsage }),

  theme: "dark",
  toggleTheme: () => {
    const next = get().theme === "dark" ? "light" : "dark";
    set({ theme: next });
    document.documentElement.classList.toggle("dark", next === "dark");
  },
}));
