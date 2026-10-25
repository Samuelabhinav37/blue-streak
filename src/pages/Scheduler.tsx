import { useEffect, useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, Trash2, CalendarClock, RefreshCw } from "lucide-react";
import { useAppStore, ScheduledTask } from "../store/useAppStore";
import TaskModal from "../components/TaskModal";

const actionLabel: Record<string, string> = {
  ScanAndClean: "Scan & Clean",
  RunRules: "Run Organizer Rules",
  EmptyRecycleBin: "Empty Recycle Bin",
};

const freqLabel: Record<string, string> = {
  Daily: "Daily",
  Weekly: "Weekly",
  OnLogin: "On Login",
};

export default function Scheduler() {
  const { scheduledTasks, setScheduledTasks } = useAppStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [statusMsg, setStatusMsg] = useState("");

  const loadTasks = useCallback(async () => {
    try {
      const tasks = await invoke<ScheduledTask[]>("list_scheduled_tasks");
      setScheduledTasks(tasks);
    } catch (_) {}
  }, [setScheduledTasks]);

  useEffect(() => {
    loadTasks();
  }, [loadTasks]);

  const createTask = async (task: ScheduledTask) => {
    setStatusMsg("Creating task…");
    try {
      await invoke("create_scheduled_task", { task });
      setModalOpen(false);
      setStatusMsg("Task created");
      loadTasks();
    } catch (err) {
      setStatusMsg(`Error: ${err}`);
    }
  };

  const deleteTask = async (name: string) => {
    try {
      await invoke("delete_scheduled_task", { name });
      setStatusMsg(`Deleted "${name}"`);
      loadTasks();
    } catch (err) {
      setStatusMsg(`Error: ${err}`);
    }
  };

  const emptyBin = async () => {
    setStatusMsg("Emptying Recycle Bin…");
    try {
      await invoke("empty_recycle_bin");
      setStatusMsg("Recycle Bin emptied");
    } catch (err) {
      setStatusMsg(`Error: ${err}`);
    }
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="px-6 py-4 border-b border-surface-600 bg-surface-800 flex items-center justify-between shrink-0">
        <div>
          <h2 className="text-sm font-semibold text-white">Scheduled Maintenance</h2>
          {statusMsg && <p className="text-xs text-slate-400 mt-0.5">{statusMsg}</p>}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={loadTasks}
            className="p-1.5 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 transition-colors"
            title="Refresh"
          >
            <RefreshCw size={14} />
          </button>
          <button
            onClick={emptyBin}
            className="px-3 py-1.5 rounded-lg bg-orange-600/20 hover:bg-orange-600/30 text-orange-400 text-xs font-medium transition-colors"
          >
            Empty Recycle Bin
          </button>
          <button
            onClick={() => setModalOpen(true)}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-accent hover:bg-accent-hover text-white text-xs font-medium transition-colors"
          >
            <Plus size={13} />
            New Task
          </button>
        </div>
      </div>

      {/* Task list */}
      <div className="flex-1 overflow-y-auto p-5">
        {scheduledTasks.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center">
            <CalendarClock size={40} className="text-slate-600 mb-3" />
            <p className="text-slate-500 text-sm">No scheduled tasks yet.</p>
            <button
              onClick={() => setModalOpen(true)}
              className="mt-4 flex items-center gap-1.5 px-4 py-2 rounded-lg bg-accent hover:bg-accent-hover text-white text-sm"
            >
              <Plus size={13} />
              Create First Task
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
            {scheduledTasks.map((task) => (
              <div
                key={task.name}
                className="bg-surface-800 border border-surface-600 rounded-xl p-4 space-y-3"
              >
                {/* Name + actions */}
                <div className="flex items-start justify-between">
                  <div>
                    <h3 className="text-sm font-semibold text-white">{task.name}</h3>
                    <p className="text-xs text-slate-400 mt-0.5">
                      {actionLabel[task.action] ?? task.action}
                    </p>
                  </div>
                  <button
                    onClick={() => deleteTask(task.name)}
                    className="p-1.5 rounded hover:bg-red-500/20 text-slate-400 hover:text-red-400 transition-colors shrink-0"
                  >
                    <Trash2 size={13} />
                  </button>
                </div>

                {/* Chips */}
                <div className="flex flex-wrap gap-1.5">
                  <span className="text-xs px-2 py-0.5 rounded-full bg-violet-500/20 text-violet-300 font-medium">
                    {freqLabel[task.frequency] ?? task.frequency}
                  </span>
                  {task.time && (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-slate-600/40 text-slate-300">
                      {task.time}
                    </span>
                  )}
                  {task.dayOfWeek && (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-slate-600/40 text-slate-300">
                      {task.dayOfWeek}
                    </span>
                  )}
                  {task.status && (
                    <span className="text-xs px-2 py-0.5 rounded-full bg-emerald-500/10 text-emerald-400">
                      {task.status}
                    </span>
                  )}
                </div>

                {/* Times */}
                <div className="text-xs text-slate-500 space-y-0.5">
                  {task.nextRun && <p>Next run: {task.nextRun}</p>}
                  {task.lastRun && <p>Last run: {task.lastRun}</p>}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {modalOpen && (
        <TaskModal
          task={null}
          onSave={createTask}
          onClose={() => setModalOpen(false)}
        />
      )}
    </div>
  );
}
