// The two uses of the pedal dialog: wrapping a selection in a new pedal, and editing the pedal being opened.
import type { CatalogItem } from "../generated/protocol";
import type { PNode } from "../model";
import { candidatesFor } from "../lib/pedal";
import type { Dialog, Editing } from "../hooks/usePedals";
import { PedalDialog, type PedalForm } from "./PedalDialog";

interface Props {
  dialog: Dialog;
  editing: Editing | null;
  nodes: PNode[];
  catalog: Map<string, CatalogItem>;
  onCreate: (ids: string[], f: PedalForm) => void;
  onSaveEdit: (f: PedalForm) => void;
  onCancel: () => void;
}

export function PedalDialogs({ dialog, editing, nodes, catalog, onCreate, onSaveEdit, onCancel }: Props) {
  if (dialog?.mode === "create") {
    return (
      <PedalDialog
        title="Create pedal"
        submitLabel="Create"
        initial={{ name: "", category: "utility", description: "" }}
        candidates={candidatesFor(nodes.filter((n) => dialog.ids.includes(n.id)), catalog)}
        knobs={[]}
        onSubmit={(f) => onCreate(dialog.ids, f)}
        onCancel={onCancel}
      />
    );
  }
  if (dialog?.mode === "edit" && editing) {
    return (
      <PedalDialog
        title={`Pedal «${editing.name}»`}
        submitLabel="Save pedal"
        initial={editing}
        candidates={candidatesFor(nodes, catalog)}
        knobs={editing.knobs}
        onSubmit={onSaveEdit}
        onCancel={onCancel}
      />
    );
  }
  return null;
}

