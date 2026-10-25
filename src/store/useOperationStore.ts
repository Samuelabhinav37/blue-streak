import { create } from "zustand";

export type OperationType = "delete" | "move" | "archive";

export interface FileOperation {
  id: string;
  type: OperationType;
  /** original path before the operation */
  originalPath: string;
  /** destination path (for move/archive) */
  newPath?: string;
  sizeBytes: number;
  timestamp: number;
  /** Whether the file went to Recycle Bin (can be undone) or was permanently deleted */
  permanent: boolean;
}

interface OperationState {
  history: FileOperation[];
  cursor: number; // index of the last applied operation (-1 = nothing applied)

  push: (op: Omit<FileOperation, "id" | "timestamp">) => void;
  undo: () => FileOperation | null;
  redo: () => FileOperation | null;
  canUndo: () => boolean;
  canRedo: () => boolean;
  clear: () => void;
}

export const useOperationStore = create<OperationState>((set, get) => ({
  history: [],
  cursor: -1,

  push(op) {
    set((s) => {
      // Discard any redo-able future when a new action is taken
      const trimmed = s.history.slice(0, s.cursor + 1);
      const newOp: FileOperation = {
        ...op,
        id: `${Date.now()}-${Math.random().toString(36).slice(2)}`,
        timestamp: Date.now(),
      };
      return {
        history: [...trimmed, newOp],
        cursor: trimmed.length, // points at the new op
      };
    });
  },

  undo() {
    const { history, cursor } = get();
    if (cursor < 0) return null;
    const op = history[cursor];
    set({ cursor: cursor - 1 });
    return op;
  },

  redo() {
    const { history, cursor } = get();
    const next = cursor + 1;
    if (next >= history.length) return null;
    const op = history[next];
    set({ cursor: next });
    return op;
  },

  canUndo: () => get().cursor >= 0,
  canRedo: () => get().cursor + 1 < get().history.length,
  clear: () => set({ history: [], cursor: -1 }),
}));
