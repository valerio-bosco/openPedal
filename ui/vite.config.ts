import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In sviluppo (`npm run dev`) il WebSocket viene girato a `openpedal ui` sulla porta 7777.
// Se openpedal non gira, l'interfaccia usa le fixture (modalità mock).
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: { "/ws": { target: "ws://127.0.0.1:7777", ws: true } },
  },
  build: { outDir: "dist", emptyOutDir: true },
});
