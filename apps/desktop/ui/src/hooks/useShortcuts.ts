// Keyboard shortcuts: Ctrl+D duplicate, Ctrl+Z undo, Ctrl+Shift+Z / Ctrl+Y redo (Delete is handled by React Flow).
import { useEffect } from "react";

interface Actions {
  duplicate: () => void;
  undo: () => void;
  redo: () => void;
}

export function useShortcuts({ duplicate, undo, redo }: Actions) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA") return;
      const mod = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      if (mod && key === "d") {
        e.preventDefault();
        duplicate();
      } else if (mod && key === "z" && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if (mod && (key === "y" || (key === "z" && e.shiftKey))) {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [duplicate, undo, redo]);
}
