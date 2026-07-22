# MANIFOLD

**A factory planner for [Satisfactory](https://www.satisfactorygame.com/) where the map is the plan.**

Most planners give you a spreadsheet: type an item, get ratios. MANIFOLD gives
you the actual world map. Factories are pins placed on real terrain, claiming
real resource nodes with real purities; belts, pipes, trains, trucks and drones
run between them over real distances; power is a second network on the same
map. Everything re-solves live as you drag — machine counts, clocks, belt
saturation, power margins, train math — so the plan you draw is the factory
you'll actually build.

**▶ Try it in your browser — nothing to install:**
**https://manifold-app.up.railway.app/**

Your plan persists in the browser between visits. The desktop app (below) keeps
plans in local files and can re-read your save automatically.

---

## Why a Satisfactory player would want this

- **Plan on the world, not in a table.** Claim the iron node you're actually
  standing next to. The extraction ceiling, the distance to your smelters, and
  the belt tier you'll need all come from the map — so "can one Mk.3 belt feed
  this?" is answered before you pour a single foundation.
- **Import your save.** Drop in your `.sav` and your running factories appear
  as the ◆ *built* layer — machines clustered into factories, clocks and counts
  as they really are. Plan expansions on top of it; re-import later and the
  differences arrive as a reviewable drift report, never a silent overwrite.
- **Let the wizard do the chain math.** Press `P`, ask for "Iron Plate at
  25/min", and the global solver plans the whole supply chain — reusing any
  surplus your empire already exports before building anything new, claiming
  nodes, sizing machines, sourcing power — then hands you a proposal you can
  accept, trim, or reject. One undo step either way.
- **Trains answered, not guessed.** Pick RAIL on a long route and the math
  block shows round trip, dwell, headway, throughput — and the headline number:
  **how many trains you need** for the demand you type in.
- **Honest numbers.** Deficits, belt bottlenecks and power brownouts surface in
  the audit drawer (`TAB`) the moment they exist. The solver names the binding
  constraint when you hit a ceiling instead of quietly clamping.
- **Run several worlds.** The DATA menu's EMPIRES section keeps every
  playthrough as its own named plan — switch, rename, or spin up a fresh one
  without losing anything.

Everything works offline. No accounts, no telemetry; your save file never
leaves your machine (the web version parses it in your browser).

---

## Install

### Web (easiest)

Open **https://manifold-app.up.railway.app/** — that's it. Plans persist in
your browser (IndexedDB). To use your own game version's recipes and import
saves, load your game's catalog once via the DATA menu (steps below).

### Windows desktop

1. Download **`MANIFOLD.exe`** from the
   [latest release](https://github.com/jon-kloss/Conveyancer/releases/latest).
2. Run it. It's a portable single file — no installer. (Windows SmartScreen may
   warn because the binary is unsigned: *More info → Run anyway*. The only
   dependency is Microsoft WebView2, preinstalled on Windows 10/11.)
3. Your plans live in `%APPDATA%\dev.ficsit.planner\`.

To load your full game catalog on desktop, point the app at your install's
Docs file before launching (PowerShell example):

```powershell
$env:FICSIT_DOCS_JSON = "C:\Program Files (x86)\Steam\steamapps\common\Satisfactory\CommunityResources\Docs\en-US.json"
.\MANIFOLD.exe
```

(Older game builds call it `Docs.json` in the same folder. Without it, a small
bundled demo catalog keeps the app fully usable for a first look.)

### macOS / Linux

No prebuilt binaries yet — the web version is the recommended way, or build
from source (see **For developers** below; the desktop shell builds with
`cargo build -p app --features prod --release` after `pnpm build`).

---

## First flight — your first ten minutes

When you open MANIFOLD on an empty plan, a first-run card offers three doors:

- **`N` — Place your first factory** if you want to start from scratch.
- **`P` — Plan a supply chain** if you want the wizard to build one for you.
- **`S` — Import a save** if you already have a running world.

A good first session, step by step:

1. **(Web only) Load your game data first.** Open **DATA ▾** (top right) →
   **① Upload Docs.json** and pick
   `…\steamapps\common\Satisfactory\CommunityResources\Docs\en-US.json`
   from your game install. This is your game version's real recipe catalog —
   the app enforces doing this before a save import so every machine in your
   save can be recognized.
2. **Import your save** — DATA ▾ → **② Import save**, pick your `.sav` (find
   them in `%LOCALAPPDATA%\FactoryGame\Saved\SaveGames\`). Your factories
   appear on the map as ◆ built, clustered and auto-named, with their real
   machines, clocks and power. (No save? Skip this — press `N` and place a
   factory anywhere instead.)
3. **Claim a node.** Search a resource in the top bar (or `Ctrl+K`), press
   Enter to jump to a node, pick a miner in its drawer and **CLAIM** it for a
   factory. A tether links node → factory, and the ore flows along it.
4. **Ask the wizard for something.** Press `P`, type an item ("iron plate"),
   set a rate, **SOLVE**. Review what it wants to build — every row has a live
   cost/power impact, and it reuses surplus your empire already makes — then
   **ACCEPT**. The new factory lands on the map as ◇ planned.
5. **Connect factories.** **Right-click-drag** from one factory pin to another
   to draw a route. The popover picks the item and suggests the transport for
   the distance — belt for short hops, rail for long hauls (with the full
   train math), pipes automatically for fluids.
6. **Open a factory.** Click its pin → **OPEN FACTORY** to see the machine
   graph: groups with counts and clocks, ports, belts with live saturation.
   Drag the output target and watch the chain re-solve; the clock buttons
   consolidate machines (fewer at 250%) or spread them (more at 50%).
7. **Check the audit.** Press `TAB` — saturation, deficits and power margins
   for the whole empire, live. The overview panel (top left) aggregates every
   resource; click a row to see who makes and eats it.
8. **Start a second world** anytime: DATA ▾ → EMPIRES → type a name →
   **+ CREATE**. Switch between empires from the same menu; nothing is lost.

**Keys worth knowing:** `N` place factory · `P` wizard · `F` frame all ·
`TAB` audit · `1/2/3/4` flow/power/node/terrain overlays · `Ctrl+K` search ·
`Ctrl+Z` undo (everything, solver included) · `Esc` back/deselect ·
`Enter` dive into selection.

### Web vs desktop

|  | Web | Windows desktop |
|---|---|---|
| Install | none — [open the app](https://manifold-app.up.railway.app/) | portable `MANIFOLD.exe` from [Releases](https://github.com/jon-kloss/Conveyancer/releases/latest) |
| Game catalog | upload `Docs.json`/`en-US.json` in the DATA menu | `FICSIT_DOCS_JSON` env var (or the bundled demo catalog) |
| Plans stored | in your browser (IndexedDB) | `%APPDATA%\dev.ficsit.planner\` (one `.ficsit` file per empire) |
| Save re-sync | re-pick the file (or a retained handle in Chrome/Edge) | remembers the save path; optional auto-sync timer |

---

## For developers

MANIFOLD (formerly FICSIT Planner) is a Tauri 2 shell + React 19 renderer over
a Rust core that owns canonical state, solvers, and persistence. Two
invariants shape everything: **Rust owns canonical state** (the renderer is a
projection patched by events), and **solves never move geometry** — numbers
change, cards don't. Every mutation is one undoable step, with solver
write-backs folded into the causing command's entry.

`docs/04-sdd.md` is the authoritative system design; `DECISIONS.md` records
every judgment call beyond the docs; `BACKLOG.md` holds deferred work.

### Layout

```
crates/planner-core   domain model, canonical state, commands, undo log,
                      proposals, transport math
crates/solver         T0 ratio propagation + T1 LP (good_lp/microlp)
crates/solver-wasm    wasm-pack wrapper over T0 for renderer drag frames
crates/gamedata       Docs.json parser (real installs + bundled fixture),
                      world-node snapshot, asset-provider trait
crates/persist        world.ficsit plan file (SQLite WAL: entities, undo
                      journal, advisor cards, mutes)
crates/app            Session (edit → solve → commit), global solver/wizard,
                      T2, save importer, advisor, chat, empires registry,
                      Tauri 2 shell, headless dev-bridge, token generator
crates/web            wasm Session wrapper for the browser build
renderer/             React 19 + Zustand + Leaflet + React Flow + import worker
docs/                 the handoff: SDD (authoritative), UI spec, Addendum A
design-reference/     pixel-accurate mockups (open the .dc.html in a browser)
fixtures/saves/       real .sav files (Dunarr-076, 269, Another-1-2)
```

### Prerequisites

- Rust (stable; developed on 1.94) — https://rustup.rs
- Node 22+ and pnpm 10+ — `npm i -g pnpm`
- `wasm-pack` only if you touch `crates/solver` or the web session closure
  (the built pkgs are committed, CI gates on source-hash drift)
- Linux desktop shell only: WebKitGTK 4.1 dev packages
  (`libwebkit2gtk-4.1-dev libgtk-3-dev` or your distro's equivalent)

### Headless dev flow (recommended — no display or GTK needed)

The `dev-bridge` binary exposes the *identical* command surface as the Tauri
shell over HTTP; the renderer can't tell the difference.

```sh
# terminal 1 — the real Rust core on port 8791
cargo run -p app --no-default-features --bin dev-bridge

# terminal 2 — the renderer on port 5173 (proxies /api → 8791)
cd renderer && pnpm install && pnpm dev
```

Open http://localhost:5173. The plan persists to `dev-world.ficsit` in the
repo root by default; sibling `*.ficsit` files appear as switchable empires.

### The desktop app (from source)

Debug builds point the window at the vite dev server, so run both:

```sh
cd renderer && pnpm dev             # terminal 1
cargo run -p app --features shell   # terminal 2 (needs WebKitGTK on Linux)
```

For a self-contained binary (embedded renderer, no vite), build the renderer
first and use the `prod` feature — plain `--release` is **not** enough, because
Tauri decides dev-vs-production by feature, not by profile:

```sh
cd renderer && pnpm build
cargo build -p app --features prod --release
```

Every merge to `main` publishes the portable Windows exe as a GitHub Release
(auto-tagged `v0.1.<build#>`); branch pushes upload the same exe as a run
artifact. Pushing a `v*` tag by hand releases under that exact version.

### Environment variables

| Variable | Meaning | Default |
|---|---|---|
| `FICSIT_PLAN` | Plan-file path (dev-bridge only); its directory is the empires registry | `dev-world.ficsit` |
| `FICSIT_BRIDGE_PORT` | dev-bridge port | `8791` |
| `FICSIT_DOCS_JSON` | Path to the game's `Docs.json`/`en-US.json` — loads the full catalog (UTF-16LE handled) | bundled fixture |
| `FICSIT_GAME_BUILD` | Build label stored with extracted gamedata | `fixture` |
| `FICSIT_AI_KEY` | Model API key; absent ⇒ `AI OFFLINE` and the heuristic engine feeds the same advisor/chat surfaces | unset |

Without a game install, the bundled fixture (the Modular Frame chain plus
belts, splitters, coal power, fracking, and one alternate recipe) keeps every
feature working offline. Imported saves that reference recipes outside the
fixture render as ◆ built factories but report `solve_error` until a real
Docs.json is configured — import is enrichment, never load-bearing.

### Testing

```sh
# Rust: unit + integration (solver golden cases, session flows, proposals,
# import/drift, advisor gating, empires) — headless, no GUI deps
cargo test --workspace --exclude app && cargo test -p app --no-default-features

# lint + format (CI enforces both)
cargo fmt --all --check
cargo clippy --workspace --exclude app -- -D warnings
cargo clippy -p app --no-default-features -- -D warnings

# renderer typecheck + unit tests
cd renderer && pnpm typecheck && pnpm test

# end-to-end: every phase exit criterion through the real UI against the real
# core (starts its own dev-bridge on a throwaway plan + vite)
cd renderer && pnpm exec playwright test
# in a container with a preinstalled browser:
#   PW_EXECUTABLE=/path/to/chromium pnpm exec playwright test

# visual sweep of the map surface (screenshots + HTML report)
cd renderer && pnpm exec playwright test --config playwright.visual.config.ts
cd renderer && node e2e-visual/report.mjs
```

The e2e suite runs serially against one shared backend: phase specs build on
each other (the phase-2 empire feeds phase-3's deficits, phase-4's import
feeds phase-5's advisor).

### Regenerating artifacts

- **Design tokens** are defined once in `crates/app/src/tokens.rs`:
  `cargo run -p app --no-default-features --bin gen-tokens` rewrites
  `renderer/src/tokens/`. CI fails on drift. No hex value ships outside the
  token system.
- **T0 WASM** (after touching `crates/solver`): `scripts/regen-wasm.sh`; **web
  session WASM** (after touching its crate closure): `scripts/regen-web-wasm.sh`.
  Both stamp the sources that fed them; CI fails on drift (`… check`).
- **Icons**: `node scripts/fetch-icons.mjs <path/to/Docs.json>` vendors the
  community 64px icon set for every catalog class and refreshes the manifest.
- **Demo seed** (a small empire with routes, a coal grid, and floors — good
  for screenshots and manual poking): start the bridge on a *fresh* plan plus
  vite, then `cd renderer && node seed.mjs`.

### Data, assets, licensing

- `.sav` parsing uses the community-maintained
  `@etothepii/satisfactory-file-parser` in a Web Worker; unknown/modded
  classes are quarantined and counted, never silently dropped. Parse failure
  degrades to manual entry — no dead ends.
- The vendored 64px icon set (greeny/SatisfactoryTools; game content
  © Coffee Stain Studios — see `NOTICE.md`) renders inside the chip tiles;
  any class without an icon keeps the monogram chip (honest degradation).
  Map tiles await the same licensing review (`DECISIONS.md`).
- The bundled world-node snapshot is a hand-assembled subset with plausible
  elevations; the full community snapshot swap-in is backlogged behind the
  same review.

### Where to read more

- `docs/04-sdd.md` — the authoritative system design (architecture, solver
  tiers and budgets, import pipeline, AI layer, phasing).
- `docs/02-ui-spec.md` + `docs/03-addendum-a.md` — every screen and the
  binding interaction principles (status grammar ◇◈◆, "orange is a verb",
  routes-are-entities, the solver budget contract).
- `design-reference/FICSIT Planner — Foundations.dc.html` — the pixel mocks.
- `DECISIONS.md` — every deviation/extension, one line each, with doc refs.
- `BACKLOG.md` — deferred work, tagged v1.1/v2.

## License

The code is dual-licensed under **MIT OR Apache-2.0** — pick either
([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). Contributions
are accepted under the same terms (see [CONTRIBUTING.md](CONTRIBUTING.md)).
Bundled community/game-derived assets are third-party content covered by
[NOTICE.md](NOTICE.md), not the project license.

MANIFOLD is a fan-made planning tool and is not affiliated with Coffee Stain
Studios. Satisfactory and its assets are © Coffee Stain Studios.
