// The React Flow canvas with the graph, the minimap and the drop target of the palette.
import { Background, BackgroundVariant, Controls, MiniMap, ReactFlow } from "@xyflow/react";
import type { CatalogItem } from "../generated/protocol";
import { CATEGORY_COLOR, type PNode } from "../model";
import type { GraphEditor } from "../hooks/useGraphEditor";
import { PedalNodeMemo } from "./PedalNode";
import { DRAG_MIME } from "./Palette";

const nodeTypes = { pedal: PedalNodeMemo };

interface Props {
  graph: GraphEditor;
  catalog: Map<string, CatalogItem>;
  /** False until the first `state` arrives: a loading message is shown. */
  ready: boolean;
  toast: (text: string) => void;
  onEnterPedal: (node: PNode) => void;
  onNodeMenu: (x: number, y: number, node: string) => void;
  onEdgeMenu: (x: number, y: number, edge: string) => void;
  onCloseMenu: () => void;
}

export function Canvas({ graph, catalog, ready, toast, onEnterPedal, onNodeMenu, onEdgeMenu, onCloseMenu }: Props) {
  const { flow, addNode, removeEdges } = graph;

  const onDrop = (e: React.DragEvent) => {
    const kind = e.dataTransfer.getData(DRAG_MIME);
    if (!kind) return;
    e.preventDefault();
    addNode(kind, flow.screenToFlowPosition({ x: e.clientX, y: e.clientY }));
  };

  return (
    <div className="canvas" ref={graph.wrapper} onDrop={onDrop} onDragOver={(e) => e.preventDefault()}>
      {!ready && <div className="loading">Connecting to openPedal…</div>}
      <ReactFlow
        nodes={graph.nodes}
        edges={graph.edges}
        nodeTypes={nodeTypes}
        onNodesChange={graph.onNodesChange}
        onEdgesChange={graph.onEdgesChange}
        onConnect={graph.onConnect}
        onConnectEnd={(_, st) => {
          if (st.toNode && !st.isValid) toast("That cable is not valid: it would create a cycle or already exists.");
        }}
        isValidConnection={graph.isValidConnection}
        onNodeDoubleClick={(_, n) => onEnterPedal(n)}
        onNodeContextMenu={(e, n) => {
          e.preventDefault();
          onNodeMenu(e.clientX, e.clientY, n.id);
        }}
        onEdgeContextMenu={(e, edge) => {
          e.preventDefault();
          onEdgeMenu(e.clientX, e.clientY, edge.id);
        }}
        onEdgeDoubleClick={(_, edge) => removeEdges([edge.id])}
        edgesReconnectable
        onReconnectStart={graph.onReconnectStart}
        onReconnect={graph.onReconnect}
        onReconnectEnd={graph.onReconnectEnd}
        onPaneClick={onCloseMenu}
        colorMode="dark"
        fitView
        minZoom={0.2}
        deleteKeyCode={["Delete", "Backspace"]}
        multiSelectionKeyCode="Shift"
        selectionKeyCode={null}
        selectionOnDrag
        panOnDrag={[1, 2]}
        defaultEdgeOptions={{ type: "default", animated: false }}
        proOptions={{ hideAttribution: true }}
      >
        <Background variant={BackgroundVariant.Dots} gap={22} size={1.4} />
        <Controls showInteractive={false} />
        <MiniMap
          pannable
          zoomable
          nodeColor={(n) => CATEGORY_COLOR[catalog.get((n as PNode).data.kind)?.category ?? "utility"]}
          maskColor="rgba(10,10,14,0.7)"
        />
      </ReactFlow>
    </div>
  );
}
