// Scheletro (fase 0): mostra il grafo d'esempio delle fixture con nodi generici.
// Il task C1 lo sostituisce con l'editor completo (vedi docs/PIANO.md §7).
import { Background, Controls, MiniMap, ReactFlow, type Edge, type Node } from "@xyflow/react";
import stateFixture from "./fixtures/state.json";
import type { StateMsg } from "./protocol";

const state = stateFixture as unknown as StateMsg;

const nodes: Node[] = state.graph.node.map((n) => ({
  id: n.id,
  position: { x: n.pos[0], y: n.pos[1] },
  data: { label: `${state.catalog.find((c) => c.kind === n.type)?.label ?? n.type} (${n.id})` },
}));

const edges: Edge[] = state.graph.edge.map((e) => ({ id: `${e.from}->${e.to}`, source: e.from, target: e.to }));

export default function App() {
  return (
    <div className="app">
      <ReactFlow defaultNodes={nodes} defaultEdges={edges} colorMode="dark" fitView>
        <Background />
        <Controls />
        <MiniMap />
      </ReactFlow>
    </div>
  );
}
