import { useEffect, useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  Plus, Play, Pencil, Trash2, ToggleLeft, ToggleRight,
  Upload, Download,
} from "lucide-react";
import { useAppStore, OrganizerRule } from "../store/useAppStore";
import RuleModal from "../components/RuleModal";

function actionBadge(action: string) {
  const map: Record<string, string> = {
    Move: "bg-blue-500/20 text-blue-300",
    Copy: "bg-green-500/20 text-green-300",
    Archive: "bg-orange-500/20 text-orange-300",
  };
  return map[action] ?? "bg-slate-500/20 text-slate-300";
}

export default function Organizer() {
  const { rules, setRules } = useAppStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingRule, setEditingRule] = useState<OrganizerRule | null>(null);
  const [statusMsg, setStatusMsg] = useState("");

  const loadRules = useCallback(async () => {
    try {
      const r = await invoke<OrganizerRule[]>("get_rules");
      setRules(r);
    } catch (_) {}
  }, [setRules]);

  useEffect(() => {
    loadRules();
  }, [loadRules]);

  const persistRules = async (updated: OrganizerRule[]) => {
    setRules(updated);
    try {
      await invoke("save_rules", { rules: updated });
    } catch (err) {
      setStatusMsg(`Save error: ${err}`);
    }
  };

  const openNew = () => {
    setEditingRule(null);
    setModalOpen(true);
  };

  const openEdit = (rule: OrganizerRule) => {
    setEditingRule(rule);
    setModalOpen(true);
  };

  const handleSave = async (rule: OrganizerRule) => {
    const exists = rules.findIndex((r) => r.id === rule.id);
    const updated =
      exists >= 0
        ? rules.map((r) => (r.id === rule.id ? rule : r))
        : [...rules, rule];
    await persistRules(updated);
    setModalOpen(false);
  };

  const deleteRule = async (id: string) => {
    await persistRules(rules.filter((r) => r.id !== id));
  };

  const toggleRule = async (id: string) => {
    await persistRules(
      rules.map((r) => (r.id === id ? { ...r, enabled: !r.enabled } : r))
    );
  };

  const runAllRules = async () => {
    setStatusMsg("Running…");
    try {
      const result = await invoke<{ processed: number; errors: string[] }>(
        "run_rules",
        { ruleIds: [] }
      );
      setStatusMsg(
        `Done — ${result.processed} file(s) processed${result.errors.length ? `, ${result.errors.length} error(s)` : ""}`
      );
    } catch (err) {
      setStatusMsg(`Error: ${err}`);
    }
  };

  // ── Marketplace: import / export ──────────────────────────────────────────

  const exportAllRules = async () => {
    if (rules.length === 0) return;
    const dest = await save({ filters: [{ name: "JSON", extensions: ["json"] }], defaultPath: "cleansweep-rules.json" });
    if (!dest) return;
    try {
      const count = await invoke<number>("export_rules", { ruleIds: [], destPath: dest });
      setStatusMsg(`Exported ${count} rule(s) to ${dest.split(/[\\/]/).pop()}`);
    } catch (err) {
      setStatusMsg(`Export error: ${err}`);
    }
  };

  const importRules = async () => {
    const src = await open({ filters: [{ name: "JSON", extensions: ["json"] }], multiple: false });
    if (!src || typeof src !== "string") return;
    try {
      const imported = await invoke<OrganizerRule[]>("import_rules_from_file", { filePath: src });
      const merged = [...rules, ...imported];
      await invoke("save_merged_rules", { rules: merged });
      setRules(merged);
      setStatusMsg(`Imported ${imported.length} rule(s)`);
    } catch (err) {
      setStatusMsg(`Import error: ${err}`);
    }
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="px-6 py-4 border-b border-surface-600 bg-surface-800 flex items-center justify-between shrink-0">
        <div>
          <h2 className="text-sm font-semibold text-white">Organize Rules</h2>
          {statusMsg && <p className="text-xs text-slate-400 mt-0.5">{statusMsg}</p>}
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={importRules}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 text-xs font-medium transition-colors"
            title="Import rules from a JSON file"
          >
            <Upload size={12} />
            Import
          </button>
          <button
            onClick={exportAllRules}
            disabled={rules.length === 0}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-surface-600 hover:bg-surface-500 text-slate-300 text-xs font-medium disabled:opacity-40 transition-colors"
            title="Export rules to a JSON file"
          >
            <Download size={12} />
            Export
          </button>
          <button
            onClick={runAllRules}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-emerald-600/20 hover:bg-emerald-600/30 text-emerald-400 text-xs font-medium transition-colors"
          >
            <Play size={12} />
            Run All
          </button>
          <button
            onClick={openNew}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-accent hover:bg-accent-hover text-white text-xs font-medium transition-colors"
          >
            <Plus size={13} />
            New Rule
          </button>
        </div>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-y-auto p-4">
        {rules.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center">
            <div className="text-slate-600 mb-3 text-4xl">⚙</div>
            <p className="text-slate-500 text-sm">
              No rules yet. Create one to auto-organize your files.
            </p>
            <button
              onClick={openNew}
              className="mt-4 flex items-center gap-1.5 px-4 py-2 rounded-lg bg-accent hover:bg-accent-hover text-white text-sm"
            >
              <Plus size={13} />
              Create First Rule
            </button>
          </div>
        ) : (
          <div className="border border-surface-600 rounded-xl overflow-hidden">
            <table className="w-full text-sm">
              <thead className="bg-surface-700 border-b border-surface-600">
                <tr>
                  <th className="text-left px-4 py-2.5 text-xs font-semibold text-slate-400">Name</th>
                  <th className="text-left px-4 py-2.5 text-xs font-semibold text-slate-400">Source</th>
                  <th className="text-left px-4 py-2.5 text-xs font-semibold text-slate-400">Destination</th>
                  <th className="text-left px-4 py-2.5 text-xs font-semibold text-slate-400">Action</th>
                  <th className="text-left px-4 py-2.5 text-xs font-semibold text-slate-400">Extensions</th>
                  <th className="text-right px-4 py-2.5 text-xs font-semibold text-slate-400">Controls</th>
                </tr>
              </thead>
              <tbody>
                {rules.map((rule, i) => (
                  <tr
                    key={rule.id}
                    className={`border-b border-surface-600 last:border-0 ${
                      i % 2 === 0 ? "bg-surface-800" : "bg-surface-700/50"
                    } ${!rule.enabled ? "opacity-50" : ""}`}
                  >
                    <td className="px-4 py-3 text-white font-medium">{rule.name}</td>
                    <td className="px-4 py-3 text-slate-400 truncate max-w-xs" title={rule.sourceFolder}>
                      {rule.sourceFolder.split(/[\\/]/).slice(-2).join("/")}
                    </td>
                    <td className="px-4 py-3 text-slate-400 truncate max-w-xs" title={rule.destination}>
                      {rule.destination.split(/[\\/]/).slice(-2).join("/")}
                    </td>
                    <td className="px-4 py-3">
                      <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${actionBadge(rule.action)}`}>
                        {rule.action}
                      </span>
                    </td>
                    <td className="px-4 py-3 text-xs text-slate-400">
                      {rule.conditions.extensions.length > 0
                        ? rule.conditions.extensions.map((e) => `.${e}`).join(", ")
                        : <span className="text-slate-600">any</span>}
                    </td>
                    <td className="px-4 py-3">
                      <div className="flex items-center justify-end gap-1">
                        <button
                          onClick={() => toggleRule(rule.id)}
                          className="p-1.5 rounded hover:bg-surface-600 text-slate-400 hover:text-white transition-colors"
                          title={rule.enabled ? "Disable" : "Enable"}
                        >
                          {rule.enabled ? (
                            <ToggleRight size={15} className="text-emerald-400" />
                          ) : (
                            <ToggleLeft size={15} />
                          )}
                        </button>
                        <button
                          onClick={() => openEdit(rule)}
                          className="p-1.5 rounded hover:bg-surface-600 text-slate-400 hover:text-white transition-colors"
                        >
                          <Pencil size={13} />
                        </button>
                        <button
                          onClick={() => deleteRule(rule.id)}
                          className="p-1.5 rounded hover:bg-red-500/20 text-slate-400 hover:text-red-400 transition-colors"
                        >
                          <Trash2 size={13} />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {modalOpen && (
        <RuleModal
          rule={editingRule}
          onSave={handleSave}
          onClose={() => setModalOpen(false)}
        />
      )}
    </div>
  );
}
