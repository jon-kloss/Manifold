import { defineConfig } from "@playwright/test";
import fs from "node:fs";

// The e2e suite drives the real Rust core through the dev-bridge — the same
// command surface the Tauri shell uses. A fresh plan file per run.
const planFile = "/tmp/ficsit-e2e-world.ficsit";
for (const f of [planFile, `${planFile}.bak`, `${planFile}-wal`, `${planFile}-shm`]) {
  try {
    fs.rmSync(f);
  } catch {
    /* fresh already */
  }
}

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: 1, // one shared backend = serialized specs
  use: {
    baseURL: "http://localhost:5173",
    viewport: { width: 1920, height: 1080 },
    launchOptions: process.env.PW_EXECUTABLE ? { executablePath: process.env.PW_EXECUTABLE } : {},
  },
  webServer: [
    {
      command: `cargo run -p app --no-default-features --bin dev-bridge`,
      cwd: "..",
      port: 8791,
      reuseExistingServer: false,
      env: { FICSIT_PLAN: planFile },
      timeout: 180_000,
    },
    {
      command: "pnpm dev",
      port: 5173,
      reuseExistingServer: true,
      timeout: 60_000,
    },
  ],
});
