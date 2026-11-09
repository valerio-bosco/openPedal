// The editor: wires the server connection, the graph editor, history, presets and pedals to the layout.
import { useCallback, useRef, useState } from "react";
import { ContextMenu } from "./components/ContextMenu";
import { Canvas } from "./components/Canvas";
import { EditorContext } from "./components/PedalNode";
import { Palette } from "./components/Palette";
import { PedalDialogs } from "./components/PedalDialogs";
import { Toasts } from "./components/Toasts";
import { MainToolbar, PedalToolbar } from "./components/Toolbar";
import { TopBar } from "./components/TopBar";
import { useContextMenu } from "./hooks/useContextMenu";
import { useEngineSync } from "./hooks/useEngineSync";
import { useGraphEditor } from "./hooks/useGraphEditor";
import { useHistory } from "./hooks/useHistory";
import { usePedals, type Editing } from "./hooks/usePedals";
import { usePresets } from "./hooks/usePresets";
import { useServer } from "./hooks/useServer";
import { useShortcuts } from "./hooks/useShortcuts";
import { useToasts } from "./hooks/useToasts";

export function Editor() {
  const { toasts, toast } = useToasts();
  const server = useServer(toast);
  const [graphName, setGraphName] = useState("");
  const [editing, setEditing] = useState<Editing | null>(null);
  const editingRef = useRef(editing);
  editingRef.current = editing;

  const graph = useGraphEditor({
    catalog: server.catalog,
    editingRef,
    send: server.send,
    hold: server.hold,
    release: server.release,
    sendParam: server.sendParam,
    toast,
  });
  const { nodes, edges, selected, tidy, duplicate } = graph;
  // The main graph's undo / redo is the core's (shared with the iPad); inside a pedal it is local.
  const history = useHistory(nodes, edges, !!editing);
  const engine = useEngineSync({ server, graph, graphName, setGraphName, editing, editingRef });
  const presets = usePresets({ server, nodes, edges, graphName, setGraphName, toast });
  const pedals = usePedals({ server, graph, history, graphName, toast, markSynced: engine.markSynced, editing, setEditing });
  const contextMenu = useContextMenu({ editing: !!editing, enterPedal: pedals.enterPedal, ...graph });

  const restoreLocal = useCallback(
    (s: { nodes: typeof nodes; edges: typeof edges } | undefined) => {
      if (!s) return;
      graph.setNodes(s.nodes);
      graph.setEdges(s.edges);
    },
    [graph.setNodes, graph.setEdges],
  );
  const { send } = server;
  const undo = useCallback(() => {
    if (editingRef.current) restoreLocal(history.undo());
    else send({ type: "undo" });
  }, [history.undo, restoreLocal, send]);
  const redo = useCallback(() => {
    if (editingRef.current) restoreLocal(history.redo());
    else send({ type: "redo" });
  }, [history.redo, restoreLocal, send]);
  const duplicateSelection = useCallback(() => duplicate(), [duplicate]);
  useShortcuts({ duplicate: duplicateSelection, undo, redo });

  const ready = server.catalogItems.length > 0;
  const selCount = selected.length;
  const edit = {
    canUndo: editing ? history.canUndo : server.canUndo,
    canRedo: editing ? history.canRedo : server.canRedo,
    canDelete: selCount + graph.selectedEdgeCount > 0,
    onUndo: undo,
    onRedo: redo,
    onDelete: graph.deleteSelection,
    onTidy: tidy,
  };

  return (
    <EditorContext.Provider value={graph.pedalContext}>
      <div className="app">
        <TopBar
          status={server.status}
          presets={server.presets}
          graphName={graphName}
          dirty={server.dirty && !editing}
          audio={server.audio}
          meters={server.meters}
          disabledPresets={!ready || !!editing}
          onLoadPreset={presets.loadPreset}
          onSave={presets.savePreset}
          onDelete={presets.deletePreset}
          extra={
            editing ? (
              <PedalToolbar
                {...edit}
                pedalName={editing.name}
                onDetails={() => pedals.setDialog({ mode: "edit" })}
                onSave={() => pedals.savePedalEdit()}
                onExit={pedals.exitPedal}
              />
            ) : (
              <MainToolbar
                {...edit}
                selectedCount={selCount}
                onNew={graph.newGraph}
                onDuplicate={duplicateSelection}
                onCreatePedal={() => pedals.setDialog({ mode: "create", ids: selected.map((n) => n.id) })}
              />
            )
          }
        />
        <div className="main">
          <Palette catalog={server.catalogItems} onAdd={graph.addNode} onDeletePedal={pedals.deletePedal} />
          <Canvas
            graph={graph}
            catalog={server.catalog}
            ready={ready}
            toast={toast}
            onEnterPedal={pedals.enterPedal}
            onNodeMenu={contextMenu.openNode}
            onEdgeMenu={contextMenu.openEdge}
            onCloseMenu={contextMenu.close}
          />
        </div>
        <PedalDialogs
          dialog={pedals.dialog}
          editing={editing}
          nodes={nodes}
          catalog={server.catalog}
          onCreate={pedals.createPedal}
          onSaveEdit={pedals.savePedalEdit}
          onCancel={() => pedals.setDialog(null)}
        />
        {contextMenu.menu && (
          <ContextMenu x={contextMenu.menu.x} y={contextMenu.menu.y} items={contextMenu.items(contextMenu.menu)} onClose={contextMenu.close} />
        )}
        <Toasts toasts={toasts} />
      </div>
    </EditorContext.Provider>
  );
}
