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
});
