// Buttons shown in the top bar: one set for the main graph, another inside a composite pedal.
interface EditProps {
  canUndo: boolean;
  canRedo: boolean;
  /** Pedals or cables are selected. */
  canDelete: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onDelete: () => void;
  onTidy: () => void;
}

function EditButtons({ canUndo, canRedo, canDelete, onUndo, onRedo, onDelete, onTidy }: EditProps) {
  return (
    <span className="btn-group">
      <button className="icon" onClick={onUndo} disabled={!canUndo} title="Undo (Ctrl+Z)" aria-label="Undo">
        ↶
      </button>
      <button className="icon" onClick={onRedo} disabled={!canRedo} title="Redo (Ctrl+Shift+Z or Ctrl+Y)" aria-label="Redo">
        ↷
      </button>
      <button onClick={onDelete} disabled={!canDelete} title="Delete the selected pedals and cables (Del)">
        Delete
      </button>
      <button onClick={onTidy} title="Arrange the pedals in columns in signal order">
        Tidy up
      </button>
    </span>
  );
}

interface MainProps extends EditProps {
  selectedCount: number;
  onNew: () => void;
  onDuplicate: () => void;
  onCreatePedal: () => void;
}

export function MainToolbar({ selectedCount, onNew, onDuplicate, onCreatePedal, ...edit }: MainProps) {
  return (
    <>
      <button onClick={onNew} title="Start from scratch: only Guitar and Amp (Ctrl+Z to go back)">
        New
      </button>
      <EditButtons {...edit} />
      <button onClick={onDuplicate} disabled={selectedCount === 0} title="Ctrl+D">
        Duplicate
      </button>
      <button
        className={selectedCount > 0 ? "primary" : undefined}
        disabled={selectedCount === 0}
        title="Wrap the selected nodes in a new pedal"
        onClick={onCreatePedal}
      >
        Create pedal
      </button>
    </>
  );
}

interface PedalProps extends EditProps {
  pedalName: string;
  onDone: () => void;
  onCancel: () => void;
}

/** Inside a pedal's inner graph: "Done" goes back to the designer with the changes, "Cancel" without. */
export function PedalToolbar({ pedalName, onDone, onCancel, ...edit }: PedalProps) {
  return (
    <>
      <span className="crumb">Inner graph of «{pedalName}»</span>
      <EditButtons {...edit} />
      <button className="primary" onClick={onDone} title="Back to the pedal designer with these changes">
        Done
      </button>
      <button onClick={onCancel} title="Back to the pedal designer, dropping the changes made here">
        Cancel
      </button>
    </>
  );
}
