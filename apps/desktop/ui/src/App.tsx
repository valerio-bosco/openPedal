import { ReactFlowProvider } from "@xyflow/react";
import { Editor } from "./Editor";

export default function App() {
  return (
    <ReactFlowProvider>
      <Editor />
    </ReactFlowProvider>
  );
}
