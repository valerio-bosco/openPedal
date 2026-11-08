import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development (`npm run dev`) the WebSocket is forwarded to `openpedal ui` on port 7777.
// If openpedal is not running, the interface uses the fixtures (mock mode).
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: { "/ws": { target: "ws://127.0.0.1:7777", ws: true } },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
