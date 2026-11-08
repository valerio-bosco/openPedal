// Presets: load, save under a name, delete.
import type { Edge } from "@xyflow/react";
import { flowToGraph, type PNode } from "../model";
import type { useServer } from "./useServer";

interface Deps {
  server: ReturnType<typeof useServer>;
  nodes: PNode[];
  edges: Edge[];
  graphName: string;
  setGraphName: (name: string) => void;
  toast: (text: string) => void;
}

export function usePresets({ server, nodes, edges, graphName, setGraphName, toast }: Deps) {
  const { send, presets } = server;

  const loadPreset = (name: string) => send({ type: "load_preset", name });

  const savePreset = () => {
    const name = window.prompt("Preset name", graphName || "my-sound")?.trim();
    if (!name) return;
    setGraphName(name);
    send({ type: "save_preset", name, graph: flowToGraph(name, nodes, edges) });
    toast(`Preset «${name}» saved`);
  };

  const deletePreset = () => {
    if (!presets.includes(graphName)) return;
    if (!window.confirm(`Delete preset «${graphName}»?\nThe file presets/${graphName}.toml will be deleted (the current sound stays).`)) return;
    send({ type: "delete_preset", name: graphName });
    toast(`Preset «${graphName}» deleted`);
  };

  return { loadPreset, savePreset, deletePreset };
}
