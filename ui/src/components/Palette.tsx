import { useMemo, useState } from "react";
import type { CatalogItem, Category } from "../protocol";
import { CATEGORY_COLOR, CATEGORY_LABEL, CATEGORY_ORDER } from "../model";

export const DRAG_MIME = "application/x-openpedal-kind";

interface Props {
  catalog: CatalogItem[];
  onAdd: (kind: string) => void;
  onDeletePedal: (item: CatalogItem) => void;
}

export function Palette({ catalog, onAdd, onDeletePedal }: Props) {
  const [query, setQuery] = useState("");
  const [closed, setClosed] = useState<Set<string>>(new Set());

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const map = new Map<Category, CatalogItem[]>();
    for (const c of catalog) {
      if (c.category === "io") continue;
      if (q && !`${c.label} ${c.kind} ${c.description ?? ""}`.toLowerCase().includes(q)) continue;
      map.set(c.category, [...(map.get(c.category) ?? []), c]);
    }
    return CATEGORY_ORDER.filter((c) => map.has(c)).map((c) => [c, map.get(c)!] as const);
  }, [catalog, query]);

  return (
    <aside className="palette">
      <input
        className="palette-search"
        type="search"
        placeholder="Search pedals…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      <div className="palette-hint">Drag onto the canvas, or double-click.</div>
      <div className="palette-list">
        {groups.map(([cat, items]) => {
          const open = query !== "" || !closed.has(cat);
          return (
            <section key={cat}>
              <button
                className="palette-cat"
                onClick={() =>
                  setClosed((s) => {
                    const n = new Set(s);
                    if (n.has(cat)) n.delete(cat);
                    else n.add(cat);
                    return n;
                  })
                }
              >
                <span className="cat-dot" style={{ background: CATEGORY_COLOR[cat] }} />
                {CATEGORY_LABEL[cat]}
                <span className="cat-count">{items.length}</span>
                <span className="cat-arrow">{open ? "▾" : "▸"}</span>
              </button>
              {open &&
                items.map((it) => (
                  <div
                    key={it.kind}
                    className="palette-item"
                    style={{ ["--cat" as string]: CATEGORY_COLOR[cat] }}
                    draggable
                    onDragStart={(e) => {
                      e.dataTransfer.setData(DRAG_MIME, it.kind);
                      e.dataTransfer.effectAllowed = "copy";
                    }}
                    onDoubleClick={() => onAdd(it.kind)}
                    title={it.description ?? it.label}
                  >
                    <span className="led" />
                    <span className="palette-label">{it.label}</span>
                    {it.composite && (
                      <button
                        className="palette-del"
                        title="Delete this pedal"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDeletePedal(it);
                        }}
                      >
                        ✕
                      </button>
                    )}
                  </div>
                ))}
            </section>
          );
        })}
        {groups.length === 0 && <div className="palette-empty">No pedals found.</div>}
      </div>
    </aside>
  );
}
