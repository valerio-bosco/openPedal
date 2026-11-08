import { useEffect } from "react";

export interface MenuItem {
  label: string;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick: () => void;
}

interface Props {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

/** Right-click menu: closes on an outside click, on Esc, or after a choice. */
export function ContextMenu({ x, y, items, onClose }: Props) {
  useEffect(() => {
    const close = () => onClose();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", close);
    };
  }, [onClose]);

  // stays inside the window even near the edges
  const left = Math.min(x, window.innerWidth - 220);
  const top = Math.min(y, window.innerHeight - items.length * 34 - 16);

  return (
    <div className="ctx-menu" style={{ left, top }} onPointerDown={(e) => e.stopPropagation()} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it) => (
        <button
          key={it.label}
          className={it.danger ? "danger" : undefined}
          disabled={it.disabled}
          onClick={() => {
            onClose();
            it.onClick();
          }}
        >
          <span>{it.label}</span>
          {it.hint && <kbd>{it.hint}</kbd>}
        </button>
      ))}
    </div>
  );
}
