import { useState, useEffect } from "react";
import { X, FolderOpen, Plus } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { OrganizerRule, RuleConditions, OrganizerAction } from "../store/useAppStore";

function newId() {
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

const emptyConditions = (): RuleConditions => ({
  extensions: [],
  minSizeMb: null,
  maxSizeMb: null,
  olderThanDays: null,
  namePatternRegex: null,
});

const emptyRule = (): OrganizerRule => ({
  id: newId(),
  name: "",
  sourceFolder: "",
  conditions: emptyConditions(),
  destination: "",
  action: "Move",
  enabled: true,
});

interface RuleModalProps {
  rule: OrganizerRule | null; // null = new rule
  onSave: (rule: OrganizerRule) => void;
  onClose: () => void;
}

export default function RuleModal({ rule, onSave, onClose }: RuleModalProps) {
  const [form, setForm] = useState<OrganizerRule>(rule ?? emptyRule());
  const [extInput, setExtInput] = useState("");

  useEffect(() => {
    setForm(rule ?? emptyRule());
    setExtInput("");
  }, [rule]);

  const set = <K extends keyof OrganizerRule>(key: K, value: OrganizerRule[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const setCond = <K extends keyof RuleConditions>(key: K, value: RuleConditions[K]) =>
    setForm((f) => ({ ...f, conditions: { ...f.conditions, [key]: value } }));

  const pickFolder = async (field: "sourceFolder" | "destination") => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") set(field, dir);
  };

  const addExt = () => {
    const e = extInput.trim().toLowerCase().replace(/^\./, "");
    if (e && !form.conditions.extensions.includes(e)) {
      setCond("extensions", [...form.conditions.extensions, e]);
    }
    setExtInput("");
  };

  const removeExt = (ext: string) =>
    setCond(
      "extensions",
      form.conditions.extensions.filter((e) => e !== ext)
    );

  const handleSave = () => {
    if (!form.name || !form.sourceFolder || !form.destination) return;
    onSave(form);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-surface-800 border border-surface-600 rounded-xl w-full max-w-lg mx-4 shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-surface-600">
          <h2 className="font-semibold text-white text-sm">
            {rule ? "Edit Rule" : "New Rule"}
          </h2>
          <button onClick={onClose} className="text-slate-400 hover:text-white">
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-4 max-h-[70vh] overflow-y-auto">
          {/* Name */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Rule Name *</label>
            <input
              value={form.name}
              onChange={(e) => set("name", e.target.value)}
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              placeholder="e.g. Archive old documents"
            />
          </div>

          {/* Source folder */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Source Folder *</label>
            <div className="flex gap-2">
              <input
                value={form.sourceFolder}
                onChange={(e) => set("sourceFolder", e.target.value)}
                className="flex-1 bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
                placeholder="C:\Users\..."
              />
              <button
                onClick={() => pickFolder("sourceFolder")}
                className="px-3 py-2 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 text-sm"
              >
                <FolderOpen size={14} />
              </button>
            </div>
          </div>

          {/* Destination */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Destination Folder *</label>
            <div className="flex gap-2">
              <input
                value={form.destination}
                onChange={(e) => set("destination", e.target.value)}
                className="flex-1 bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
                placeholder="C:\Organized\..."
              />
              <button
                onClick={() => pickFolder("destination")}
                className="px-3 py-2 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 text-sm"
              >
                <FolderOpen size={14} />
              </button>
            </div>
          </div>

          {/* Action */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Action</label>
            <select
              value={form.action}
              onChange={(e) => set("action", e.target.value as OrganizerAction)}
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
            >
              <option value="Move">Move</option>
              <option value="Copy">Copy</option>
              <option value="Archive">Archive to ZIP</option>
            </select>
          </div>

          {/* Extensions */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">
              File Extensions (leave empty for all)
            </label>
            <div className="flex gap-2">
              <input
                value={extInput}
                onChange={(e) => setExtInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && addExt()}
                className="flex-1 bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
                placeholder=".pdf, .docx…"
              />
              <button
                onClick={addExt}
                className="px-3 py-2 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300"
              >
                <Plus size={14} />
              </button>
            </div>
            {form.conditions.extensions.length > 0 && (
              <div className="flex flex-wrap gap-1.5 mt-2">
                {form.conditions.extensions.map((ext) => (
                  <span
                    key={ext}
                    className="flex items-center gap-1 text-xs bg-accent/20 text-accent-light px-2 py-0.5 rounded-full"
                  >
                    .{ext}
                    <button onClick={() => removeExt(ext)}>
                      <X size={10} />
                    </button>
                  </span>
                ))}
              </div>
            )}
          </div>

          {/* Size range */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs text-slate-400 mb-1 block">Min Size (MB)</label>
              <input
                type="number"
                value={form.conditions.minSizeMb ?? ""}
                onChange={(e) =>
                  setCond("minSizeMb", e.target.value ? parseFloat(e.target.value) : null)
                }
                className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
                placeholder="—"
              />
            </div>
            <div>
              <label className="text-xs text-slate-400 mb-1 block">Max Size (MB)</label>
              <input
                type="number"
                value={form.conditions.maxSizeMb ?? ""}
                onChange={(e) =>
                  setCond("maxSizeMb", e.target.value ? parseFloat(e.target.value) : null)
                }
                className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
                placeholder="—"
              />
            </div>
          </div>

          {/* Older than */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Older Than (days)</label>
            <input
              type="number"
              value={form.conditions.olderThanDays ?? ""}
              onChange={(e) =>
                setCond("olderThanDays", e.target.value ? parseInt(e.target.value) : null)
              }
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              placeholder="—"
            />
          </div>

          {/* Regex */}
          <div>
            <label className="text-xs text-slate-400 mb-1 block">Name Pattern (regex)</label>
            <input
              value={form.conditions.namePatternRegex ?? ""}
              onChange={(e) =>
                setCond("namePatternRegex", e.target.value || null)
              }
              className="w-full bg-surface-700 border border-surface-500 rounded-lg px-3 py-2 text-sm text-white outline-none focus:border-accent"
              placeholder="e.g. ^report_.*\.pdf$"
            />
          </div>

          {/* Enabled */}
          <div className="flex items-center gap-2">
            <input
              type="checkbox"
              id="rule-enabled"
              checked={form.enabled}
              onChange={(e) => set("enabled", e.target.checked)}
              className="w-4 h-4 accent-violet-500"
            />
            <label htmlFor="rule-enabled" className="text-sm text-slate-300">
              Rule enabled
            </label>
          </div>
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
            disabled={!form.name || !form.sourceFolder || !form.destination}
            className="px-4 py-2 rounded-lg text-sm bg-accent hover:bg-accent-hover text-white font-medium disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
          >
            Save Rule
          </button>
        </div>
      </div>
    </div>
  );
}
