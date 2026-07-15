#!/usr/bin/env node
// Drift guard: renderer/src/lib/iconManifest.json ↔ renderer/public/icons/.
// House pattern, same as the gen-tokens "in sync" CI step: the manifest is a
// PROMISE — every listed class must have its PNG (ItemIcon's manifest gate
// renders an <img> for it, so a missing file is a guaranteed broken image),
// and every vendored PNG must be listed (an untracked file is dead weight the
// renderer can never reach). Bidirectional set-diff; any drift prints the
// named {missing, untracked} diffs and exits 1.
//
//   node scripts/check-icon-manifest.mjs
//
// Paths resolve from this script's own location, so it runs from anywhere
// (CI invokes it from the repo root).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const MANIFEST = path.join(ROOT, "renderer/src/lib/iconManifest.json");
const ICON_DIR = path.join(ROOT, "renderer/public/icons");

const manifest = new Set(JSON.parse(fs.readFileSync(MANIFEST, "utf8")));
const files = new Set(
  fs
    .readdirSync(ICON_DIR)
    .filter((f) => f.endsWith(".png"))
    .map((f) => path.basename(f, ".png")),
);

const missing = [...manifest].filter((c) => !files.has(c)).sort();
const untracked = [...files].filter((c) => !manifest.has(c)).sort();

if (missing.length > 0 || untracked.length > 0) {
  console.error(JSON.stringify({ missing, untracked }, null, 2));
  process.exit(1);
}
console.log(`icon manifest in sync — ${manifest.size} classes`);
