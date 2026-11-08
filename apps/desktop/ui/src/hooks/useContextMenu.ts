// Right-click menu on pedals and cables.
import { useState } from "react";
import { PEDAL_PREFIX } from "../generated/protocol";
import { isFixed, type PNode } from "../model";
import type { MenuItem } from "../components/ContextMenu";

type Menu = { x: number; y: number; target: { node: string } | { edge: string } } | null;

interface Actions {
  nodes: PNode[];
  /** True inside a pedal: pedals cannot be entered from there. */
  editing: boolean;
  enterPedal: (node: PNode) => void;
  duplicate: (only?: string[]) => void;
  disconnect: (id: string) => void;
  removeNodes: (ids: string[]) => void;
  removeEdges: (ids: string[]) => void;
}

export function useContextMenu({ nodes, editing, enterPedal, duplicate, disconnect, removeNodes, removeEdges }: Actions) {
  const [menu, setMenu] = useState<Menu>(null);

  const items = (m: NonNullable<Menu>): MenuItem[] => {
    if ("edge" in m.target) {
      const id = m.target.edge;
      return [{ label: "Delete cable", hint: "Del", danger: true, onClick: () => removeEdges([id]) }];
    }
    const node = nodes.find((n) => n.id === (m.target as { node: string }).node);
    if (!node) return [];
    const fixed = isFixed(node.data.kind);
    const list: MenuItem[] = [];
    if (node.data.kind.startsWith(PEDAL_PREFIX) && !editing) {
      list.push({ label: "Enter pedal", hint: "double click", onClick: () => enterPedal(node) });
    }
    list.push(
      {
        label: "Duplicate",
        hint: "Ctrl+D",
        disabled: fixed,
        onClick: () => duplicate([node.id]),
      },
      { label: "Disconnect all cables", onClick: () => disconnect(node.id) },
      { label: "Delete pedal", hint: "Del", danger: true, disabled: fixed, onClick: () => removeNodes([node.id]) },
    );
    return list;
  };

  return {
    menu,
    items,
    close: () => setMenu(null),
    openNode: (x: number, y: number, node: string) => setMenu({ x, y, target: { node } }),
    openEdge: (x: number, y: number, edge: string) => setMenu({ x, y, target: { edge } }),
  };
}
