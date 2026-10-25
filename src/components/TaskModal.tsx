import { useState, useEffect } from "react";
import { X } from "lucide-react";
import { ScheduledTask, TaskAction, ScheduleFrequency } from "../store/useAppStore";

const emptyTask = (): ScheduledTask => ({
  name: "",
  action: "ScanAndClean",
  frequency: "Daily",
  time: "02:00",
  dayOfWeek: "MON",
  lastRun: null,
  nextRun: null,
  status: "",
});

interface TaskModalProps {
  task: ScheduledTask | null;
  onSave: (task: ScheduledTask) => void;
  onClose: () => void;
}

export default function TaskModal({ task, onSave, onClose }: TaskModalProps) {
  const [form, setForm] = useState<ScheduledTask>(task ?? emptyTask());

  useEffect(() => {
    setForm(task ?? emptyTask());
  }, [task]);

  const set = <K extends keyof ScheduledTask>(key: K, value: ScheduledTask[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const handleSave = () => {
    if (!form.name.trim()) return;
    onSave(form);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-surface-800 border border-surface-600 rounded-xl w-full max-w-md mx-4 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-surface-600">
          <h2 className="font-semibold text-white text-sm">
            {task ? "Edit Task" : "New Scheduled Task"}
          </h2>
          <button onClick={onClose} className="text-slate-400 hover:text-white">
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-4">
          {/* Name */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Task Name *</label>
            <input
              value={form.name}
              onChange={(e) => set("name", e.target.value)}
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              placeholder="e.g. Nightly Cleanup"
            />
          </div>

          {/* Action */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Action</label>
            <select
              value={form.action}
              onChange={(e) => set("action", e.target.value as TaskAction)}
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
            >
              <option value="ScanAndClean">Scan &amp; Clean</option>
              <option value="RunRules">Run Organizer Rules</option>
              <option value="EmptyRecycleBin">Empty Recycle Bin</option>
            </select>
          </div>

          {/* Frequency */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Frequency</label>
            <select
              value={form.frequency}
              onChange={(e) => set("frequency", e.target.value as ScheduleFrequency)}
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
            >
              <option value="Daily">Daily</option>
              <option value="Weekly">Weekly</option>
              <option value="OnLogin">On Login</option>
            </select>
          </div>

          {/* Time */}
          {form.frequency !== "OnLogin" && (
            <div>
              <label className="text-xs text-slate-400 mb-1 block">Time (HH:MM)</label>
              <input
                type="time"
                value={form.time ?? "02:00"}
                onChange={(e) => set("time", e.target.value)}
                className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              />
            </div>
          )}

          {/* Day of week (weekly only) */}
          {form.frequency === "Weekly" && (
            <div>
              <label className="text-xs text-slate-400 mb-1 block">Day of Week</label>
              <select
                value={form.dayOfWeek ?? "MON"}
                onChange={(e) => set("dayOfWeek", e.target.value)}
                className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              >
                {["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"].map((d) => (
                  <option key={d} value={d}>
                    {d}
                  </option>
                ))}
              </select>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-2 px-5 py-3 border-t border-surface-600">
          <button
            onClick={onClose}
            className="px-4 py-2 rounded-lg text-sm text-slate-400 hover:text-white hover:bg-surface-600 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={!form.name.trim()}
            className="px-4 py-2 rounded-lg text-sm bg-accent hover:bg-accent-hover text-white font-medium disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            Save Task
          </button>
        </div>
      </div>
    </div>
  );
}
