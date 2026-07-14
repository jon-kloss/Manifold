import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev: the renderer runs against either the Tauri shell (devUrl) or the
// headless dev-bridge (proxy below). Prod: built into the Tauri bundle.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": {
        target: `http://127.0.0.1:${process.env.FICSIT_BRIDGE_PORT ?? 8791}`,
        changeOrigin: true,
      },
    },
  },
  build: { target: "es2022" },
  // The save-parse Web Worker (src/import/parseWorker.ts) imports the heavy
  // @etothepii/satisfactory-file-parser. vite's dep scanner does not follow
  // `new Worker(new URL(...))`, so without this the parser's whole ESM tree is
  // transformed ON DEMAND the first time the worker loads — cheap on a warm dev
  // box, but pathologically slow cold on a constrained CI runner (the e2e save
  // import timed out at 120s there). Pre-bundling it at server startup makes the
  // first worker load instant. Dev-only; the production build is unaffected.
  optimizeDeps: { include: ["@etothepii/satisfactory-file-parser"] },
});
