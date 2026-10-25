import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Map, FolderOpen, ChevronRight, HardDrive, ArrowUp,
  Copy, Trash2, ExternalLink, X,
} from "lucide-react";
import { useAppStore } from "../store/useAppStore";
// ─── types ────────────────────────────────────────────────────────────────────

interface TreemapNode {
  label: string;
  path: string;
  sizeBytes: number;
  fileCount: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color: string;
  depth: number;
  children: TreemapNode[];
}

interface JunkFile {
  path: string;
  sizeBytes: number;
  category: string;
  ageDays: number;
}

// ─── helpers ──────────────────────────────────────────────────────────────────

function fmtSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

// Continuous HSL color: green (small) → yellow → red (large)
function sizeColor(bytes: number): string {
  const gb = bytes / (1024 ** 3);
  // 0 GB → hue 120 (green), 1 GB → hue 0 (red), clamp
  const hue = Math.max(0, 120 - Math.round(gb * 120));
  return `hsl(${hue}, 65%, 45%)`;
}

const CANVAS_W = 780;
const CANVAS_H = 480;

// ─── context menu ─────────────────────────────────────────────────────────────

interface CtxMenu {
  x: number;
  y: number;
  node: TreemapNode;
}

function ContextMenu({ menu, onClose }: { menu: CtxMenu; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [onClose]);

  const copyPath = () => { navigator.clipboard.writeText(menu.node.path); onClose(); };

  const openExplorer = async () => {
    try {
      // Use PowerShell to open the path in Explorer
      const target = menu.node.path;
      await invoke("open_html_report", { freedBytes: 0, files: [], periodLabel: "explorer" }).catch(() => {});
      // Best-effort: invoke a shell command via the existing tauri shell plugin
      const { Command } = await import("@tauri-apps/plugin-shell");
      await Command.create("explorer", [target]).execute().catch(() => {});
    } catch { /**/ }
    onClose();
  };

  const deleteNode = async () => {
    try {
      await invoke("delete_files", { paths: [menu.node.path], permanent: false });
    } catch { /**/ }
    onClose();
  };

  return (
    <div
      ref={ref}
      className="fixed z-50 bg-surface-800 border border-surface-600 rounded-lg shadow-xl py-1 text-xs text-slate-300 min-w-[160px]"
      style={{ top: menu.y, left: menu.x }}
    >
      <button
        onClick={openExplorer}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-surface-700 hover:text-white transition-colors"
      >
        <ExternalLink size={12} /> Open in Explorer
      </button>
      <button
        onClick={copyPath}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-surface-700 hover:text-white transition-colors"
      >
        <Copy size={12} /> Copy Path
      </button>
      <hr className="border-surface-600 my-1" />
      <button
        onClick={deleteNode}
        className="w-full flex items-center gap-2 px-3 py-1.5 hover:bg-red-900/40 hover:text-red-300 transition-colors"
      >
        <Trash2 size={12} /> Delete (to Recycle Bin)
      </button>
    </div>
  );
}

// ─── tooltip ──────────────────────────────────────────────────────────────────

interface Tooltip {
  x: number;
  y: number;
  node: TreemapNode;
}

function TooltipOverlay({ tip }: { tip: Tooltip }) {
  return (
    <div
      className="fixed z-40 pointer-events-none bg-surface-900 border border-surface-600 rounded-lg px-3 py-2 text-xs shadow-lg max-w-xs"
      style={{ top: tip.y + 14, left: tip.x + 14 }}
    >
      <p className="font-semibold text-white truncate max-w-[220px]">{tip.node.label}</p>
      <p className="text-slate-400 mt-0.5">{fmtSize(tip.node.sizeBytes)}</p>
      {tip.node.fileCount > 0 && (
        <p className="text-slate-500">{tip.node.fileCount.toLocaleString()} items</p>
      )}
    </div>
  );
}

// ─── canvas renderer (auto-switch at 5K nodes) ────────────────────────────────

const CANVAS_SWITCH_THRESHOLD = 5_000;

function TreemapCanvas({
  node,
  onSelect,
  onContextMenu,
  onTooltip,
  selected,
}: {
  node: TreemapNode;
  onSelect: (n: TreemapNode) => void;
  onContextMenu: (e: React.MouseEvent, n: TreemapNode) => void;
  onTooltip: (tip: Tooltip | null) => void;
  selected: string | null;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const nodes = node.children.length > 0 ? node.children : [node];

  // Re-draw whenever nodes or selected changes
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.clearRect(0, 0, CANVAS_W, CANVAS_H);
    ctx.fillStyle = "#0f172a";
    ctx.fillRect(0, 0, CANVAS_W, CANVAS_H);

    for (const n of nodes) {
      const x = n.x + 1;
      const y = n.y + 1;
      const w = Math.max(0, n.width - 2);
      const h = Math.max(0, n.height - 2);

      ctx.globalAlpha = selected === n.path ? 1.0 : 0.78;
      ctx.fillStyle = sizeColor(n.sizeBytes);
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, 2);
      ctx.fill();

      if (selected === n.path) {
        ctx.globalAlpha = 1;
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 2;
        ctx.stroke();
      } else {
        ctx.globalAlpha = 0.4;
        ctx.strokeStyle = "#0f172a";
        ctx.lineWidth = 1;
        ctx.stroke();
      }

      ctx.globalAlpha = 1;

      // Label: only if rect is large enough
      if (n.width > 40 && n.height > 25) {
        const fs = Math.min(13, Math.max(9, n.width / 9));
        ctx.font = `${fs}px system-ui, sans-serif`;
        ctx.fillStyle = "rgba(255,255,255,0.9)";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        const labelY = n.y + n.height / 2 - (n.height > 45 ? 7 : 0);
        const label = n.label.length > 20 ? n.label.slice(0, 18) + "…" : n.label;
        ctx.fillText(label, n.x + n.width / 2, labelY, w - 4);
      }
      // Size sub-label
      if (n.width > 40 && n.height > 45) {
        const fs2 = Math.min(11, Math.max(8, n.width / 11));
        ctx.font = `${fs2}px system-ui, sans-serif`;
        ctx.fillStyle = "rgba(255,255,255,0.55)";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(fmtSize(n.sizeBytes), n.x + n.width / 2, n.y + n.height / 2 + 9, w - 4);
      }
    }
  }, [nodes, selected]);

  // Hit-test: find the node under the mouse cursor
  const hitTest = (e: React.MouseEvent<HTMLCanvasElement>): TreemapNode | null => {
    const rect = (e.target as HTMLCanvasElement).getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    // Iterate in reverse so the last-drawn (topmost) node wins
    for (let i = nodes.length - 1; i >= 0; i--) {
      const n = nodes[i];
      if (mx >= n.x && mx <= n.x + n.width && my >= n.y && my <= n.y + n.height) {
        return n;
      }
    }
    return null;
  };

  return (
    <canvas
      ref={canvasRef}
      width={CANVAS_W}
      height={CANVAS_H}
      className="rounded-lg border border-surface-600"
      style={{ cursor: "pointer", display: "block" }}
      onClick={(e) => { const n = hitTest(e); if (n) onSelect(n); }}
      onContextMenu={(e) => { e.preventDefault(); const n = hitTest(e); if (n) onContextMenu(e, n); }}
      onMouseMove={(e) => { const n = hitTest(e); onTooltip(n ? { x: e.clientX, y: e.clientY, node: n } : null); }}
      onMouseLeave={() => onTooltip(null)}
    />
  );
}

// ─── treemap SVG ──────────────────────────────────────────────────────────────

function TreemapSvg({
  node,
  onSelect,
  onContextMenu,
  onTooltip,
  selected,
}: {
  node: TreemapNode;
  onSelect: (n: TreemapNode) => void;
  onContextMenu: (e: React.MouseEvent, n: TreemapNode) => void;
  onTooltip: (tip: Tooltip | null) => void;
  selected: string | null;
}) {
  const nodes = node.children.length > 0 ? node.children : [node];

  return (
    <svg
      width={CANVAS_W}
      height={CANVAS_H}
      className="rounded-lg overflow-hidden border border-surface-600"
      style={{ background: "#0f172a" }}
    >
      {nodes.map((n) => (
        <g
          key={n.path}
          onClick={() => onSelect(n)}
          onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, n); }}
          onMouseEnter={(e) => onTooltip({ x: e.clientX, y: e.clientY, node: n })}
          onMouseMove={(e) => onTooltip({ x: e.clientX, y: e.clientY, node: n })}
          onMouseLeave={() => onTooltip(null)}
          style={{ cursor: "pointer" }}
        >
          <rect
            x={n.x + 1}
            y={n.y + 1}
            width={Math.max(0, n.width - 2)}
            height={Math.max(0, n.height - 2)}
            fill={sizeColor(n.sizeBytes)}
            fillOpacity={selected === n.path ? 1.0 : 0.78}
            stroke={selected === n.path ? "#fff" : "#0f172a"}
            strokeWidth={selected === n.path ? 2 : 1}
            rx={2}
          />
          {/* Name label — only if rect is big enough (40×25 px threshold) */}
          {n.width > 40 && n.height > 25 && (
            <text
              x={n.x + n.width / 2}
              y={n.y + n.height / 2 - (n.height > 45 ? 7 : 0)}
              textAnchor="middle"
              dominantBaseline="middle"
              fontSize={Math.min(13, Math.max(9, n.width / 9))}
              fill="white"
              fillOpacity={0.9}
              style={{ pointerEvents: "none", userSelect: "none" }}
            >
              {n.label.length > 20 ? n.label.slice(0, 18) + "…" : n.label}
            </text>
          )}
          {/* Size sub-label */}
          {n.width > 40 && n.height > 45 && (
            <text
              x={n.x + n.width / 2}
              y={n.y + n.height / 2 + 9}
              textAnchor="middle"
              dominantBaseline="middle"
              fontSize={Math.min(11, Math.max(8, n.width / 11))}
              fill="white"
              fillOpacity={0.55}
              style={{ pointerEvents: "none", userSelect: "none" }}
            >
              {fmtSize(n.sizeBytes)}
            </text>
          )}
        </g>
      ))}
    </svg>
  );
}

// ─── auto-switching treemap renderer ─────────────────────────────────────────
// Uses SVG below CANVAS_SWITCH_THRESHOLD nodes (better DX, CSS styling).
// Auto-switches to Canvas above threshold (avoids DOM thrashing at 5K+ rects).

function TreemapRenderer(props: {
  node: TreemapNode;
  onSelect: (n: TreemapNode) => void;
  onContextMenu: (e: React.MouseEvent, n: TreemapNode) => void;
  onTooltip: (tip: Tooltip | null) => void;
  selected: string | null;
}) {
  const count = props.node.children.length > 0 ? props.node.children.length : 1;
  return count >= CANVAS_SWITCH_THRESHOLD
    ? <TreemapCanvas {...props} />
    : <TreemapSvg {...props} />;
}

// ─── file list panel ──────────────────────────────────────────────────────────

function FileListPanel({
  scanResults,
  dirPath,
  onClose,
}: {
  scanResults: JunkFile[];
  dirPath: string;
  onClose: () => void;
}) {
  const [sortBy, setSortBy] = useState<"size" | "name" | "age">("size");

  // Filter to files under the selected directory
  const files = scanResults
    .filter((f) => f.path.startsWith(dirPath))
    .sort((a, b) => {
      if (sortBy === "size") return b.sizeBytes - a.sizeBytes;
      if (sortBy === "age") return b.ageDays - a.ageDays;
      return a.path.localeCompare(b.path);
    });

  return (
    <div className="w-64 shrink-0 bg-surface-800 border border-surface-600 rounded-xl flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-surface-700">
        <span className="text-xs font-medium text-white truncate max-w-[160px]" title={dirPath}>
          {dirPath.split(/[/\\]/).pop() || dirPath}
        </span>
        <button onClick={onClose} className="text-slate-500 hover:text-white ml-2">
          <X size={12} />
        </button>
      </div>
      {/* Sort controls */}
      <div className="flex gap-1 px-2 py-1.5 border-b border-surface-700">
        {(["size", "name", "age"] as const).map((s) => (
          <button
            key={s}
            onClick={() => setSortBy(s)}
            className={`px-2 py-0.5 rounded text-[10px] capitalize transition-colors ${
              sortBy === s ? "bg-violet-600 text-white" : "text-slate-500 hover:text-white"
            }`}
          >
            {s}
          </button>
        ))}
        <span className="ml-auto text-[10px] text-slate-600">{files.length} files</span>
      </div>
      {files.length === 0 ? (
        <div className="flex-1 flex items-center justify-center text-xs text-slate-600">
          No scanned files here
        </div>
      ) : (
        <div className="overflow-y-auto flex-1">
          {files.slice(0, 200).map((f, i) => {
            const name = f.path.replace(/.*[/\\]/, "");
            return (
              <div key={i} className="flex items-center gap-2 px-3 py-1 hover:bg-surface-700 text-xs border-b border-surface-700/50">
                <span className="flex-1 truncate text-slate-300" title={f.path}>{name}</span>
                <span className="shrink-0 text-slate-500">{fmtSize(f.sizeBytes)}</span>
                <span className="shrink-0 text-slate-600 w-12 text-right">{f.ageDays}d</span>
              </div>
            );
          })}
          {files.length > 200 && (
            <div className="px-3 py-1 text-[10px] text-slate-600">…and {files.length - 200} more</div>
          )}
        </div>
      )}
    </div>
  );
}

// ─── main page ────────────────────────────────────────────────────────────────

export default function DiskMap() {
  const [rootPath, setRootPath] = useState("C:\\");
  const [tree, setTree] = useState<TreemapNode | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<TreemapNode | null>(null);
  const [error, setError] = useState("");
  // Breadcrumb stack: each element is the parent tree at that level
  const [stack, setStack] = useState<TreemapNode[]>([]);
  // Hover tooltip
  const [tooltip, setTooltip] = useState<Tooltip | null>(null);
  // Right-click context menu
  const [ctxMenu, setCtxMenu] = useState<CtxMenu | null>(null);
  // File list panel — reads from global store (populated by Scanner page)
  const { scanResults } = useAppStore();
  const [showFileList, setShowFileList] = useState(false);

  const load = useCallback(async (path: string) => {
    setLoading(true);
    setError("");
    setSelected(null);
    setStack([]);
    setShowFileList(false);
    try {
      // Use cached variant — instant if scan was already done for this root
      const result = await invoke<TreemapNode>("compute_treemap_cached", {
        rootPath: path,
        canvasWidth: CANVAS_W,
        canvasHeight: CANVAS_H,
      });
      setTree(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const browse = async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir === "string") {
      setRootPath(dir);
      load(dir);
    }
  };

  const handleSelect = (node: TreemapNode) => {
    setSelected(node);
    setShowFileList(true);
    // Drill into directories with children
    if (node.children.length > 0) {
      setStack((s) => [...s, tree!]);
      setTree(node);
    }
  };

  // Jump to a specific breadcrumb level
  const jumpTo = (index: number) => {
    if (index < 0) {
      // Jump to root
      if (stack.length > 0) {
        setTree(stack[0]);
        setStack([]);
      }
    } else {
      setTree(stack[index]);
      setStack((s) => s.slice(0, index));
    }
    setSelected(null);
    setShowFileList(false);
  };

  const handleContextMenu = (e: React.MouseEvent, node: TreemapNode) => {
    setCtxMenu({ x: e.clientX, y: e.clientY, node });
  };

  // Keyboard: Backspace = drill up, Enter = drill in, Arrows = sibling navigation
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Don't capture when an input/textarea has focus
      const tag = (e.target as HTMLElement).tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;

      if (e.key === "Backspace" && stack.length > 0) {
        e.preventDefault();
        jumpTo(stack.length - 2);
        return;
      }
      if (e.key === "Enter" && selected?.children.length) {
        e.preventDefault();
        handleSelect(selected);
        return;
      }
      // Arrow keys: navigate between siblings in the current level
      if ((e.key === "ArrowRight" || e.key === "ArrowLeft" ||
           e.key === "ArrowUp"    || e.key === "ArrowDown") && tree) {
        e.preventDefault();
        const siblings = tree.children.length > 0 ? tree.children : [];
        if (siblings.length === 0) return;

        // Find current index; default to -1 if none selected
        const currentIdx = selected
          ? siblings.findIndex((n) => n.path === selected.path)
          : -1;

        let nextIdx: number;
        if (e.key === "ArrowRight" || e.key === "ArrowDown") {
          nextIdx = currentIdx < siblings.length - 1 ? currentIdx + 1 : 0;
        } else {
          nextIdx = currentIdx > 0 ? currentIdx - 1 : siblings.length - 1;
        }
        const next = siblings[nextIdx];
        setSelected(next);
        setShowFileList(true);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stack, selected, tree]);

  // Build full breadcrumb path
  const crumbs = stack.map((n) => n.label);
  if (tree) crumbs.push(tree.label);

  return (
    <div className="flex flex-col h-full overflow-hidden p-5 gap-3" onClick={() => setCtxMenu(null)}>
      {/* Header */}
      <div className="flex items-center gap-3">
        <Map size={18} className="text-violet-400" />
        <h1 className="text-sm font-semibold text-white">Disk Map</h1>
        <div className="flex-1" />
        <div className="flex items-center gap-2">
          <input
            value={rootPath}
            onChange={(e) => setRootPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && load(rootPath)}
            className="bg-surface-700 border border-surface-600 rounded-lg px-3 py-1.5 text-xs text-slate-300 w-56 focus:outline-none focus:border-violet-500"
          />
          <button
            onClick={browse}
            className="p-1.5 rounded-lg bg-surface-700 hover:bg-surface-600 text-slate-400 hover:text-white transition-colors"
          >
            <FolderOpen size={14} />
          </button>
          <button
            onClick={() => load(rootPath)}
            disabled={loading}
            className="px-3 py-1.5 rounded-lg bg-accent hover:bg-accent-hover text-white text-xs font-medium disabled:opacity-50 transition-colors"
          >
            {loading ? "Loading…" : "Analyze"}
          </button>
        </div>
      </div>

      {/* Clickable breadcrumb — E2 */}
      {stack.length > 0 && (
        <div className="flex items-center gap-1 text-xs text-slate-400 flex-wrap">
          <button
            onClick={() => jumpTo(-1)}
            className="flex items-center gap-1 hover:text-white transition-colors"
            title="Back to root"
          >
            <HardDrive size={12} />
            Root
          </button>
          {stack.map((n, i) => (
            <span key={i} className="flex items-center gap-1">
              <ChevronRight size={11} />
              <button
                onClick={() => jumpTo(i)}
                className="hover:text-white transition-colors"
              >
                {n.label}
              </button>
            </span>
          ))}
          {tree && (
            <span className="flex items-center gap-1 text-white font-medium">
              <ChevronRight size={11} />
              {tree.label}
            </span>
          )}
          <button
            onClick={() => jumpTo(stack.length - 2)}
            className="ml-2 flex items-center gap-1 text-slate-500 hover:text-white transition-colors"
            title="Go up one level (Backspace)"
          >
            <ArrowUp size={11} />
          </button>
        </div>
      )}

      {/* Error */}
      {error && (
        <p className="text-xs text-red-400 bg-red-900/20 rounded-lg px-3 py-2">{error}</p>
      )}

      {/* Main area */}
      <div className="flex-1 flex gap-3 overflow-hidden min-h-0">
        {/* Treemap + controls */}
        <div className="flex-1 flex flex-col gap-2 min-w-0">
          {loading && (
            <div className="flex items-center justify-center h-full">
              <div className="text-slate-500 text-sm animate-pulse">Building disk map…</div>
            </div>
          )}
          {!loading && !tree && (
            <div className="flex flex-col items-center justify-center h-full text-center">
              <Map size={40} className="text-slate-600 mb-3" />
              <p className="text-slate-500 text-sm">
                Enter a path and click Analyze to see where your disk space is used.
              </p>
              <p className="text-slate-600 text-xs mt-1">
                Tip: Run a scan first — DiskMap will be instant.
              </p>
            </div>
          )}
          {!loading && tree && (
            <>
              <TreemapRenderer
                node={tree}
                onSelect={handleSelect}
                onContextMenu={handleContextMenu}
                onTooltip={setTooltip}
                selected={selected?.path ?? null}
              />
              {/* Selected info bar */}
              {selected && (
                <div className="flex items-center gap-4 px-3 py-2 bg-surface-800 rounded-lg border border-surface-700 text-xs">
                  <span className="font-medium text-white truncate max-w-xs" title={selected.path}>
                    {selected.label}
                  </span>
                  <span className="text-slate-400">{fmtSize(selected.sizeBytes)}</span>
                  <span className="text-slate-500">{selected.fileCount.toLocaleString()} items</span>
                  <span className="text-slate-600 truncate flex-1" title={selected.path}>{selected.path}</span>
                  <button
                    onClick={() => setShowFileList((v) => !v)}
                    className="shrink-0 px-2 py-1 rounded bg-surface-700 hover:bg-surface-600 text-slate-400 hover:text-white transition-colors"
                  >
                    {showFileList ? "Hide files" : "Show files"}
                  </button>
                </div>
              )}
            </>
          )}
        </div>

        {/* File list panel — E3 */}
        {showFileList && selected && (
          <FileListPanel
            scanResults={scanResults}
            dirPath={selected.path}
            onClose={() => setShowFileList(false)}
          />
        )}
      </div>

      {/* Legend */}
      <div className="flex items-center gap-5 text-[11px] text-slate-500 shrink-0">
        <span className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded-sm inline-block" style={{ background: "hsl(120,65%,45%)" }} /> Small
        </span>
        <span className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded-sm inline-block" style={{ background: "hsl(60,65%,45%)" }} /> Medium
        </span>
        <span className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded-sm inline-block" style={{ background: "hsl(0,65%,45%)" }} /> Large
        </span>
        <span className="ml-auto">Click = drill in · Arrows = select sibling · Enter = drill in · Backspace = go up · Right-click = options</span>
      </div>

      {/* Tooltip — E1 */}
      {tooltip && <TooltipOverlay tip={tooltip} />}

      {/* Context menu — E4 */}
      {ctxMenu && <ContextMenu menu={ctxMenu} onClose={() => setCtxMenu(null)} />}
    </div>
  );
}
