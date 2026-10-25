import { Sun, Moon, Bell } from "lucide-react";
import { useAppStore } from "../store/useAppStore";

const pageTitles: Record<string, string> = {
  dashboard: "Dashboard",
  scanner: "File Scanner & Cleaner",
  organizer: "Auto-Organize Rules",
  watcher: "Real-time Folder Watcher",
  scheduler: "Scheduled Maintenance",
  settings: "Settings",
};

export default function TopBar() {
  const { activePage, theme, toggleTheme } = useAppStore();

  return (
    <header className="h-12 bg-surface-800 border-b border-surface-600 flex items-center justify-between px-6 shrink-0">
      <h1 className="text-sm font-semibold text-white">
        {pageTitles[activePage] ?? activePage}
      </h1>

      <div className="flex items-center gap-2">
        <button
          onClick={toggleTheme}
          className="p-1.5 rounded-md text-slate-400 hover:text-white hover:bg-surface-600 transition-colors"
          title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
        >
          {theme === "dark" ? <Sun size={15} /> : <Moon size={15} />}
        </button>
        <button className="p-1.5 rounded-md text-slate-400 hover:text-white hover:bg-surface-600 transition-colors">
          <Bell size={15} />
        </button>
      </div>
    </header>
  );
}
