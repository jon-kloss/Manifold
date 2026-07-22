#!/usr/bin/env node
// Vendor the community 64px item/machine icon set (greeny/SatisfactoryTools,
// icons extracted from the game — content © Coffee Stain Studios; see
// NOTICE.md) for every class the gamedata parser serves to the renderer.
//
//   node scripts/fetch-icons.mjs <path/to/Docs.json>
//
// Docs.json may be UTF-16LE (real installs) or UTF-8 (the bundled fixture) —
// detected by BOM, same as crates/gamedata/src/docs.rs. Icons land in
// renderer/public/icons/<ClassName>.png and every class that HAS a file is
// recorded in renderer/src/lib/iconManifest.json (sorted, merged with any
// existing manifest — a fixture run never truncates the real-catalog run).
// A 404 is fine: the class simply stays off the manifest and the renderer
// keeps its monogram chip (honest degradation, never a broken <img>).

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const BASE = "https://raw.githubusercontent.com/greeny/SatisfactoryTools/dev/www/assets/images/items";
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ICON_DIR = path.join(ROOT, "renderer/public/icons");
const MANIFEST = path.join(ROOT, "renderer/src/lib/iconManifest.json");
const CONCURRENCY = 8;

// Sections whose classes get icons — mirrors the docs.rs native-class match
// arms (items + solver-relevant machines/belts), NOT the whole FGBuildable*
// catalog (256px machine renders are ~300KB; the 64px set stays ~6-10KB each).
const ITEM_NATIVE = new Set([
  "FGItemDescriptor",
  "FGResourceDescriptor",
  "FGItemDescriptorBiomass",
  "FGItemDescriptorNuclearFuel",
  "FGEquipmentDescriptor",
]);
const BUILDABLE_NATIVE = new Set([
  "FGBuildableManufacturer",
  "FGBuildableManufacturerVariablePower",
  "FGBuildableResourceExtractor",
  "FGBuildableWaterPump",
  "FGBuildableGeneratorFuel",
  "FGBuildableGeneratorNuclear",
  "FGBuildableGeneratorGeoThermal",
  "FGBuildableConveyorBelt",
  // Fracking wells (game-parity arc): the Pressurizer + Resource Well
  // Extractor render as machine-group cards, so they need real icons too.
  "FGBuildableFrackingActivator",
  "FGBuildableFrackingExtractor",
]);
// The graph LOGISTIC menu renders these buildables as placeable rows too.
const EXTRA_BUILDABLES = [
  "Build_ConveyorAttachmentSplitter_C",
  "Build_ConveyorAttachmentSplitterSmart_C",
  "Build_ConveyorAttachmentSplitterProgrammable_C",
  "Build_ConveyorAttachmentMerger_C",
  "Build_StorageContainerMk1_C",
  // The 4-way Pipeline Junction Cross (PR #81) — same catalog row treatment
  // as the belt splitter/merger.
  "Build_PipelineJunction_Cross_C",
];

const docsPath = process.argv[2];
if (!docsPath) {
  console.error("usage: node scripts/fetch-icons.mjs <path/to/Docs.json>");
  process.exit(1);
}

const buf = fs.readFileSync(docsPath);
const text =
  buf[0] === 0xff && buf[1] === 0xfe ? buf.toString("utf16le", 2) : buf.toString("utf8");
const sections = JSON.parse(text);

const classes = new Set(EXTRA_BUILDABLES);
for (const section of sections) {
  // ".../FactoryGame.FGItemDescriptor'" → "FGItemDescriptor"
  const fg = (section.NativeClass ?? "").split(".").pop().replace(/'/g, "");
  if (!ITEM_NATIVE.has(fg) && !BUILDABLE_NATIVE.has(fg)) continue;
  for (const c of section.Classes ?? []) {
    if (c.ClassName) classes.add(c.ClassName);
  }
}

// SatisfactoryTools files machine renders under the Desc_ stem:
// Build_AssemblerMk1_C → desc-assemblermk1-c_64.png.
function iconUrl(className) {
  const stem = className.startsWith("Build_") ? `Desc_${className.slice(6)}` : className;
  return `${BASE}/${stem.toLowerCase().replace(/_/g, "-")}_64.png`;
}

fs.mkdirSync(ICON_DIR, { recursive: true });

const fetched = [];
const skipped = [];
const missing = [];
const failed = [];

async function fetchOne(className) {
  const file = path.join(ICON_DIR, `${className}.png`);
  if (fs.existsSync(file)) {
    skipped.push(className);
    return;
  }
  const res = await fetch(iconUrl(className));
  if (res.status === 404) {
    missing.push(className);
    return;
  }
  if (!res.ok) {
    failed.push(`${className} (HTTP ${res.status})`);
    return;
  }
  fs.writeFileSync(file, Buffer.from(await res.arrayBuffer()));
  fetched.push(className);
}

const queue = [...classes].sort();
const workers = Array.from({ length: CONCURRENCY }, async () => {
  for (let cls; (cls = queue.shift()) !== undefined; ) await fetchOne(cls);
});
await Promise.all(workers);

// Manifest = union of the existing manifest and every class with a file on
// disk (merge, never truncate — the fixture is a subset of the real catalog).
let manifest = [];
try {
  manifest = JSON.parse(fs.readFileSync(MANIFEST, "utf8"));
} catch {
  /* first run */
}
const have = new Set(manifest);
for (const cls of classes) {
  if (fs.existsSync(path.join(ICON_DIR, `${cls}.png`))) have.add(cls);
}
fs.writeFileSync(MANIFEST, `${JSON.stringify([...have].sort(), null, 2)}\n`);

console.log(`classes:  ${classes.size}`);
console.log(`fetched:  ${fetched.length}`);
console.log(`skipped:  ${skipped.length} (already on disk)`);
console.log(`missing:  ${missing.length}${missing.length ? ` — ${missing.join(", ")}` : ""}`);
if (failed.length) console.log(`FAILED:   ${failed.length} — ${failed.join(", ")}`);
console.log(`manifest: ${have.size} entries → ${path.relative(ROOT, MANIFEST)}`);
process.exit(failed.length ? 1 : 0);
