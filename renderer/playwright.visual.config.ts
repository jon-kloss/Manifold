import { defineConfig } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

// Visual functional suite for the world map (renderer/e2e-visual). Separate
// from the CI suite: its own plan file, its own output dir, screenshots are
// taken explicitly by the specs and collated into an HTML report by
// e2e-visual/report.mjs. Run with:
//   pnpm exec playwright test --config playwright.visual.config.ts
const planFile = "/tmp/ficsit-visual-world.ficsit";
const outDir = path.resolve("e2e-visual/out");

// Runner main process only (mirrors playwright.config.ts): wipe the plan DB
// and the previous run's shots/results before the webServers launch.
if (!process.env.TEST_WORKER_INDEX) {
  for (const f of [planFile, `${planFile}.bak`, `${planFile}-wal`, `${planFile}-shm`]) {
    try {
      fs.rmSync(f);
    } catch {
      /* fresh already */
    }
  }
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(path.join(outDir, "shots"), { recursive: true });
}

export default defineConfig({
  testDir: "./e2e-visual",
  timeout: 90_000,
  workers: 1, // one shared backend = serialized tests, declaration order
  use: {
    baseURL: "http://localhost:5173",
    viewport: { width: 1920, height: 1080 },
    contextOptions: { reducedMotion: "reduce" },
    launchOptions: process.env.PW_EXECUTABLE ? { executablePath: process.env.PW_EXECUTABLE } : {},
  },
  webServer: [
    {
      command: `cargo run -p app --no-default-features --features bridge --bin dev-bridge`,
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
