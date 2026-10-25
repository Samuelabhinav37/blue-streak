import {
  LayoutDashboard,
  ScanSearch,
  FolderSync,
  Eye,
  CalendarClock,
  Settings,
  Zap,
  Map,
} from "lucide-react";
import { useAppStore } from "../store/useAppStore";

const navItems = [
  { id: "dashboard",  label: "Dashboard",  icon: LayoutDashboard },
  { id: "scanner",    label: "Scanner",    icon: ScanSearch },
  { id: "diskmap",    label: "Disk Map",   icon: Map },
  { id: "organizer",  label: "Organizer",  icon: FolderSync },
  { id: "watcher",    label: "Watcher",    icon: Eye },
  { id: "scheduler",  label: "Scheduler",  icon: CalendarClock },
  { id: "settings",   label: "Settings",   icon: Settings },
];

export default function Sidebar() {
  const { activePage, setActivePage } = useAppStore();

  return (
    <aside className="w-56 shrink-0 bg-surface-800 border-r border-surface-600 flex flex-col">
      {/* Logo */}
      <div className="flex items-center gap-2 px-5 py-5 border-b border-surface-600">
        <div className="w-7 h-7 rounded-lg bg-accent flex items-center justify-center">
          <Zap size={14} className="text-white" />
        </div>
        <span className="font-semibold text-white text-sm tracking-wide">
          CleanSweep
        </span>
      </div>

      {/* Nav items */}
      <nav className="flex-1 py-3 space-y-0.5 px-2">
        {navItems.map(({ id, label, icon: Icon }) => {
          const active = activePage === id;
          return (
            <button
              key={id}
              onClick={() => setActivePage(id)}
              className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors
                ${
                  active
                    ? "bg-accent text-white"
                    : "text-slate-400 hover:text-white hover:bg-surface-600"
                }`}
            >
              <Icon size={16} />
              {label}
            </button>
          );
        })}
      </nav>

      {/* Version badge */}
      <div className="px-5 py-4 border-t border-surface-600">
        <p className="text-xs text-slate-500">v0.1.0</p>
      </div>
    </aside>
  );
}
