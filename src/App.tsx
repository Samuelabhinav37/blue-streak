import type { ComponentType } from "react";
import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "./store/useAppStore";
import Sidebar from "./components/Sidebar";
import TopBar from "./components/TopBar";
import Dashboard from "./pages/Dashboard";
import Scanner from "./pages/Scanner";
import DiskMap from "./pages/DiskMap";
import Organizer from "./pages/Organizer";
import Watcher from "./pages/Watcher";
import Scheduler from "./pages/Scheduler";
import Settings from "./pages/Settings";

const pages: Record<string, ComponentType> = {
  dashboard: Dashboard,
  scanner:   Scanner,
  diskmap:   DiskMap,
  organizer: Organizer,
  watcher:   Watcher,
  scheduler: Scheduler,
  settings:  Settings,
};

export default function App() {
  const { activePage } = useAppStore();
  const Page = pages[activePage] ?? Dashboard;

  // Phase 5 (C3): on first mount, try to restore the previous scan index from
  // disk so DiskMap is available immediately without a full rescan.
  useEffect(() => {
    invoke("load_snapshot").catch(() => {/* no snapshot yet — that's fine */});
  }, []);

  return (
    <div className="flex h-screen w-screen bg-surface-900 text-white overflow-hidden select-none">
      <Sidebar />
      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
        <TopBar />
        <main className="flex-1 overflow-hidden">
          <Page />
        </main>
      </div>
    </div>
  );
}
