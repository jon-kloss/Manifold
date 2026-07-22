// Build the self-contained HTML report from out/results.jsonl + out/shots/*.png.
// Screenshots are inlined as base64 data URIs so the file opens anywhere.
//   node e2e-visual/report.mjs  →  e2e-visual/out/map-visual-test-report.html

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.join(HERE, "out");
const SHOTS = path.join(OUT, "shots");
const REPORT = path.join(OUT, "map-visual-test-report.html");

// The full test plan. `test` links a plan row to a spec test's title prefix;
// rows without one are deliberate SKIPs, shown with their reason.
const PLAN = [
  { area: "Boot & onboarding", what: "Empty empire boots to the onboarding card; SKIP lands on the world map with grid, nodes, legend, status bar.", test: "01" },
  { area: "Zoom & pan", what: "Wheel zoom updates the % readout (smooth fractional zoom), left-drag pans without side effects, F frames all factories.", test: "02" },
  { area: "Terrain overlay", what: "Toolbar button toggles the terrain/biome overlay on and off.", test: "03" },
  { area: "Node search & filter", what: "Typing live-filters resource nodes by type; clearing restores all nodes.", test: "04" },
  { area: "Node drawer & miner claim", what: "Enter on a search hit jumps to the node and opens its drawer; claiming for a factory creates the claim, tether and ceilinged IN port.", test: "05" },
  { area: "Manual factory creation", what: "N (or + FACTORY) arms placing mode; a map click drops an auto-named ◇ pin with an auto-resolved region.", test: "06" },
  { area: "Pin drag vs pan", what: "Dragging a pin commits the new position; a plain map pan never moves it.", test: "07" },
  { area: "Summary drawer & graph entry", what: "Pin click opens the summary drawer (name, region, elevation, theme); OPEN FACTORY enters the factory graph view.", test: "08" },
  { area: "Hand-built factory graph", what: "A seeded ore→ingot chain renders as cards: IN/OUT ports, SMELTER ×2 @ 80% (solver-sized for the 48/min target), belts with flow.", test: "09" },
  { area: "Wizard factory build (centerpiece)", what: "P → Iron Plate 25/min → SOLVE → review (goal ✓, Δ POWER, CREATE/CLAIM items) → ACCEPT. The materialized build is verified in the graph AND against the plan data: SMELTER ×2 @ 62.5%, CONSTRUCTOR ×2 @ 62.5%, plate OUT at 25/min, iron node claimed.", test: "10" },
  { area: "Belt route drawing", what: "Right-drag pin→pin opens the candidate popover; BELT commits; the inspector re-caps on tier change (Mk.1 → 60/min).", test: "11" },
  { area: "Route draft cancel", what: "ESC mid right-drag drops the ghost and hint; releasing produces no popover and commits nothing.", test: "12" },
  { area: "Pipe route (fluids)", what: "A Water OUT→IN pair pins the medium to PIPE (no belt/rail choice), pipe tiers only; Mk.2 → 600/min CAP.", test: "13" },
  { area: "Power line & priority switch", what: "MW OUT → consumer offers exactly one candidate (Power line); the inspector shows grid membership; + PRIORITY SWITCH drops a switch pin with its drawer; status bar shows draw vs generation.", test: "14" },
  { area: "Rail route & train answer", what: "Long distance suggests RAIL; the pre-build TRAIN ANSWER sizes consists for a typed demand; the committed inspector carries the math block; +1 consist doubles throughput.", test: "15" },
  { area: "Truck route", what: "TRUCK commits a road link with truck count + fuel spec.", test: "16" },
  { area: "Drone route", what: "DRONE commits a drone link with batteries-per-trip spec.", test: "17" },
  { area: "Geyser claim", what: "A geyser's drawer offers PLACE GEOTHERMAL (purity → MW); claiming stamps a generator factory; reopening shows GO TO GENERATOR.", test: "18" },
  { area: "Fracking well claim", what: "One satellite claims the whole well: Pressurizer + per-satellite extractor groups; the well factory's graph shows the build.", test: "19" },
  { area: "Map overlays & audit drawer", what: "FLOWS (1), POWER (2), NODES (3) overlay toggles; TAB opens the audit drawer with empire saturation/deficits.", test: "20" },
  { area: "Resource overview panel", what: "Aggregate table → full table + grids → per-item drill-down → collapse to rail and back.", test: "21" },
  { area: "DATA menu / new empire", what: "Start-new-empire arms on first click ('Click again'), wipes the whole plan on the second; onboarding returns.", test: "22" },
  { area: "Save import (built layer)", what: "Importing a .sav builds the ◆ layer + DIFF drift. Deliberately SKIPPED here: covered by CI (audit-import, phase4-import) and needs 2–5 min import waits that would dwarf the visual run.", skip: "covered by CI import specs" },
  { area: "DIFF overlay / plan drift", what: "Requires an imported built layer — SKIPPED with import (CI-covered).", skip: "needs imported save" },
  { area: "Mobile dashboard & style guides", what: "Not map surfaces — out of scope for this sweep (CI-covered elsewhere).", skip: "out of scope (not the map)" },
];

const esc = (s) => String(s ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const lines = fs.existsSync(path.join(OUT, "results.jsonl"))
  ? fs.readFileSync(path.join(OUT, "results.jsonl"), "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l))
  : [];

// Group by test title, preserving first-seen order; a retry's later verdict wins.
const byTest = new Map();
for (const e of lines) {
  if (!byTest.has(e.test)) byTest.set(e.test, { shots: [], verdict: null });
  const t = byTest.get(e.test);
  if (e.kind === "shot" && !t.shots.some((s) => s.file === e.file)) t.shots.push(e);
  if (e.kind === "verdict") t.verdict = e;
}
const tests = [...byTest.entries()].sort(([a], [b]) => a.localeCompare(b));

const counts = { passed: 0, failed: 0, skipped: PLAN.filter((p) => p.skip).length, other: 0 };
for (const [, t] of tests) {
  const s = t.verdict?.status;
  if (s === "passed") counts.passed++;
  else if (s === "failed" || s === "timedOut") counts.failed++;
  else counts.other++;
}

const badge = (status) => {
  const map = {
    passed: ["PASS", "#1d7a3e", "#e7f6ec"],
    failed: ["FAIL", "#b3261e", "#fdeceb"],
    timedOut: ["FAIL (timeout)", "#b3261e", "#fdeceb"],
    skipped: ["SKIPPED", "#8a6d00", "#fdf6dd"],
  };
  const [label, fg, bg] = map[status] ?? [String(status ?? "NO VERDICT").toUpperCase(), "#5f6368", "#eee"];
  return `<span class="badge" style="color:${fg};background:${bg};border-color:${fg}">${label}</span>`;
};

const img = (file) => {
  // prefer a downscaled JPEG twin (generated post-run) so the report stays
  // portable; fall back to the raw PNG
  const jpg = path.join(SHOTS, file.replace(/\.png$/, ".jpg"));
  if (fs.existsSync(jpg))
    return `<img loading="lazy" alt="${esc(file)}" src="data:image/jpeg;base64,${fs.readFileSync(jpg).toString("base64")}"/>`;
  const p = path.join(SHOTS, file);
  if (!fs.existsSync(p)) return `<div class="missing">screenshot missing: ${esc(file)}</div>`;
  return `<img loading="lazy" alt="${esc(file)}" src="data:image/png;base64,${fs.readFileSync(p).toString("base64")}"/>`;
};

const planRows = PLAN.map((p) => {
  const t = p.test ? tests.find(([title]) => title.startsWith(p.test)) : null;
  const status = p.skip ? "skipped" : t?.[1].verdict?.status;
  return `<tr><td class="mono">${esc(p.test ?? "—")}</td><td><b>${esc(p.area)}</b></td><td>${esc(p.what)}${p.skip ? `<div class="skipnote">Skipped: ${esc(p.skip)}</div>` : ""}</td><td>${badge(status)}</td></tr>`;
}).join("\n");

const sections = tests
  .map(([title, t]) => {
    const v = t.verdict;
    const shots = t.shots
      .map(
        (s, i) => `
      <figure>
        ${img(s.file)}
        <figcaption><span class="mono">#${i + 1}</span> ${esc(s.caption)}</figcaption>
      </figure>`,
      )
      .join("\n");
    const error = v?.error
      ? `<pre class="error">${esc(v.error)}</pre>`
      : "";
    return `
  <section id="t${title.slice(0, 2)}">
    <h2>${esc(title)} ${badge(v?.status)} <span class="dur">${v ? (v.durationMs / 1000).toFixed(1) + "s" : ""}</span></h2>
    ${error}
    ${shots || '<p class="missing">no screenshots recorded</p>'}
  </section>`;
  })
  .join("\n");

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>Conveyancer — Map Visual Test Report</title>
<style>
  :root { color-scheme: light; }
  * { box-sizing: border-box; }
  body { margin: 0; font: 15px/1.55 -apple-system, "Segoe UI", Roboto, sans-serif; color: #202124; background: #f7f8fa; }
  header { background: #14181c; color: #eceef0; padding: 28px 32px; }
  header h1 { margin: 0 0 6px; font-size: 22px; letter-spacing: .04em; }
  header .sub { color: #9aa4ad; font-size: 13px; }
  .summary { display: flex; gap: 14px; margin-top: 16px; flex-wrap: wrap; }
  .stat { background: #1f262c; border: 1px solid #2c343b; border-radius: 8px; padding: 10px 18px; text-align: center; }
  .stat b { display: block; font-size: 22px; }
  .stat.pass b { color: #46c07a; } .stat.fail b { color: #ff6b61; } .stat.skip b { color: #e5c454; }
  main { max-width: 1180px; margin: 0 auto; padding: 28px 24px 80px; }
  .badge { display: inline-block; font: 700 11px/1 monospace; letter-spacing: .08em; padding: 4px 8px; border: 1px solid; border-radius: 4px; vertical-align: middle; }
  table { width: 100%; border-collapse: collapse; background: #fff; border: 1px solid #e0e3e7; border-radius: 8px; overflow: hidden; font-size: 13.5px; }
  th, td { text-align: left; padding: 9px 12px; border-bottom: 1px solid #eceef0; vertical-align: top; }
  th { background: #f1f3f5; font-size: 12px; letter-spacing: .06em; text-transform: uppercase; color: #5f6368; }
  .mono { font-family: ui-monospace, monospace; }
  .skipnote { color: #8a6d00; font-size: 12.5px; margin-top: 3px; }
  section { margin-top: 40px; background: #fff; border: 1px solid #e0e3e7; border-radius: 10px; padding: 20px 22px; }
  section h2 { margin: 0 0 12px; font-size: 16px; }
  .dur { color: #9aa4ad; font-size: 12px; font-weight: 400; }
  figure { margin: 18px 0; }
  figure img { max-width: 100%; border: 1px solid #d6dade; border-radius: 6px; box-shadow: 0 1px 4px rgba(0,0,0,.08); }
  figcaption { margin-top: 7px; color: #444; font-size: 13.5px; }
  figcaption .mono { color: #9aa4ad; margin-right: 6px; }
  .error { background: #fdeceb; border: 1px solid #f2b8b5; border-radius: 6px; padding: 12px; white-space: pre-wrap; font-size: 12.5px; overflow-x: auto; }
  .missing { color: #b3261e; font-size: 13px; }
  h1.plan { font-size: 18px; margin: 34px 0 12px; }
</style>
</head>
<body>
<header>
  <h1>CONVEYANCER — WORLD MAP VISUAL TEST REPORT</h1>
  <div class="sub">Playwright functional sweep · ${new Date().toISOString().slice(0, 16).replace("T", " ")} UTC · every screenshot is taken at the moment the surrounding assertions held</div>
  <div class="summary">
    <div class="stat pass"><b>${counts.passed}</b>passed</div>
    <div class="stat fail"><b>${counts.failed}</b>failed</div>
    <div class="stat skip"><b>${counts.skipped}</b>skipped</div>
    <div class="stat"><b>${tests.length}</b>tests run</div>
  </div>
</header>
<main>
  <h1 class="plan">Test plan &amp; verdicts</h1>
  <table>
    <thead><tr><th>#</th><th>Area</th><th>What is verified</th><th>Verdict</th></tr></thead>
    <tbody>${planRows}</tbody>
  </table>
  ${sections}
</main>
</body>
</html>`;

fs.writeFileSync(REPORT, html);
console.log(`report: ${REPORT} (${(fs.statSync(REPORT).size / 1024 / 1024).toFixed(1)} MB, ${tests.length} tests, ${lines.filter((l) => l.kind === "shot").length} shots)`);
