import type { CSSProperties } from "react";
import { Trash2, File, FolderOpen } from "lucide-react";
import { JunkFile } from "../store/useAppStore";
import { useAppStore } from "../store/useAppStore";

const categoryColors: Record<string, string> = {
  Temp:              "bg-yellow-500/20 text-yellow-300",
  Download:          "bg-blue-500/20 text-blue-300",
  Duplicate:         "bg-orange-500/20 text-orange-300",
  EmptyDir:          "bg-slate-500/20 text-slate-300",
  Large:             "bg-red-500/20 text-red-300",
  PossibleDuplicate: "bg-amber-500/20 text-amber-300",
};

const categoryLabels: Record<string, string> = {
  Temp:              "Temp",
  Download:          "Download",
  Duplicate:         "Duplicate",
  EmptyDir:          "Empty Dir",
  Large:             "Large",
  PossibleDuplicate: "~Duplicate",
};

function fmtSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(1)} ${units[i]}`;
}

/**
 * Custom rowProps passed via react-window v2 List's `rowProps` prop.
 * `index` and `style` are injected automatically by react-window and must NOT
 * appear in this interface (they are ForbiddenKeys in react-window v2 types).
 */
export interface FileResultRowProps {
  files: JunkFile[];
}

// react-window injects `index` and `style` at runtime even though they are
// absent from FileResultRowProps. We cast them in via a local alias.
type RowRuntimeProps = FileResultRowProps & { index: number; style: CSSProperties };

export default function FileResultRow(props: FileResultRowProps) {
  const { index, style, files } = props as RowRuntimeProps;
  const { selectedFiles, toggleFileSelected } = useAppStore();
  const file = files[index];
  if (!file) return null;

  const selected = selectedFiles.has(file.path);
  const isDir = file.category === "EmptyDir";

  return (
    <div
      style={style}
      className={`flex items-center gap-3 px-4 cursor-pointer transition-colors
        ${selected ? "bg-accent/20 border border-accent/40" : "hover:bg-surface-600 border border-transparent"}
        rounded-lg mx-2`}
      onClick={() => toggleFileSelected(file.path)}
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={() => toggleFileSelected(file.path)}
        onClick={(e) => e.stopPropagation()}
        className="w-4 h-4 accent-violet-500 shrink-0"
      />

      <div className="shrink-0 text-slate-400">
        {isDir ? <FolderOpen size={15} /> : <File size={15} />}
      </div>

      <div className="flex-1 min-w-0">
        <p className="text-sm text-slate-200 truncate" title={file.path}>
          {file.path.split(/[\\/]/).pop()}
        </p>
        <p className="text-xs text-slate-500 truncate">{file.path}</p>
      </div>

      <div className="flex items-center gap-3 shrink-0">
        <span
          className={`text-xs px-2 py-0.5 rounded-full font-medium ${categoryColors[file.category] ?? "bg-slate-600 text-slate-300"}`}
          title={file.category === "PossibleDuplicate" ? "Near-duplicate (LSH fingerprint match)" : undefined}
        >
          {categoryLabels[file.category] ?? file.category}
        </span>
        <span className="text-xs text-slate-400 w-16 text-right">
          {fmtSize(file.sizeBytes)}
        </span>
        <span className="text-xs text-slate-500 w-16 text-right">
          {file.ageDays}d ago
        </span>
        <button
          onClick={(e) => {
            e.stopPropagation();
            toggleFileSelected(file.path);
          }}
          className="p-1 rounded hover:bg-red-500/20 text-slate-500 hover:text-red-400 transition-colors"
        >
          <Trash2 size={13} />
        </button>
      </div>
    </div>
  );
}
