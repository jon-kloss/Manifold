import { defineConfig } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

// Desktop save-sync e2e: the dev-bridge's /api/sync/pick returns this path in
// place of a native OS picker (unscriptable headless), so the sync flow can be
// driven end-to-end. Absolute so it resolves regardless of the bridge's cwd.
const syncSave = path.resolve("../fixtures/saves/Dunarr-076.sav");

// The e2e suite drives the real Rust core through the dev-bridge — the same
// command surface the Tauri shell uses. A fresh plan file per run.
const planFile = "/tmp/ficsit-e2e-world.ficsit";
// The config re-executes in every worker process (and on each worker restart).
// Only the runner main process — evaluated before webServers launch — may wipe
// the plan DB; a worker-side rm unlinks it out from under the live dev-bridge.
// NOTE: globalSetup is NOT a safe home for this — in Playwright 1.61 webServer
// plugins start BEFORE globalSetup runs.
if (!process.env.TEST_WORKER_INDEX) {
  for (const f of [planFile, `${planFile}.bak`, `${planFile}-wal`, `${planFile}-shm`]) {
    try {
      fs.rmSync(f);
    } catch {
      /* fresh already */
    }
  }
}

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: 1, // one shared backend = serialized specs
  use: {
    baseURL: "http://localhost:5173",
    viewport: { width: 1920, height: 1080 },
    // Run the whole suite under prefers-reduced-motion. The flow animations
    // honor it by design (CSS `animation: none` on the dash overlays + the map
    // RAF loop parks itself), so a software-rasterized CI runner stops burning
    // cores repainting animated dashes — repaint contention is what made it
    // drop mouse events mid drag-gesture (connect(), right-drag routes). The
    // motion assertions stay valid: they assert presence of `.edge-flowing`
    // et al (classes, not pixels), which reduced motion does not remove.
    contextOptions: { reducedMotion: "reduce" },
    launchOptions: process.env.PW_EXECUTABLE ? { executablePath: process.env.PW_EXECUTABLE } : {},
  },
  webServer: [
    {
      command: `cargo run -p app --no-default-features --features bridge --bin dev-bridge`,
      cwd: "..",
      port: 8791,
      reuseExistingServer: false,
      env: { FICSIT_PLAN: planFile, FICSIT_SYNC_SAVE: syncSave },
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
