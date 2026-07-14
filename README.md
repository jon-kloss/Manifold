# FICSIT Planner

A desktop factory & logistics planner for Satisfactory. Tauri 2 shell, React 19
renderer, Rust core owning canonical state, solvers, and persistence. **The map
is the source of truth** — factories are placed entities claiming real resource
nodes, connected by routes with real distances; power is a second network on
the same map; plans are living blueprints that re-solve as you drag.

All five phases of the design contract are implemented (see `docs/04-sdd.md`
§12 for the phasing table — every exit criterion is covered by the e2e suite):

| Phase | What shipped |
|---|---|
| 1 | Map home (Leaflet), factory graph (React Flow), T0 ratio + T1 LP solvers, Docs.json gamedata pipeline, single-file plan persistence with full undo |
| 2 | Inter-factory belt routes + empire recompute, audit drawer (TAB), power circuits + generator factories, live saturation/deficit/margin surfaces |
| 3 | Proposal system (reviewable, partially-acceptable change sets), global solver + supply-chain wizard (P), T2 recipe-optimization mini-proposals, priority switches |
| 4 | Rail/truck/drone route math with visible inspectors, `.sav` import (worker parse → DBSCAN clustering → ◆ built layer), re-import drift → proposals + DIFF tab |
| 5 | Ambient advisor (gated heuristics, persistent mutes), offline chat with `proposal_intent`, style guides, first-run onboarding, asset-provider abstraction |

`DECISIONS.md` records every judgment call beyond the docs; `BACKLOG.md` holds
everything deliberately deferred, tagged v1.1/v2.

## Layout

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
                      T2, save importer, advisor, chat, Tauri 2 shell,
                      headless dev-bridge, token generator
renderer/             React 19 + Zustand + Leaflet + React Flow + import worker
docs/                 the handoff: SDD (authoritative), UI spec, Addendum A
design-reference/     pixel-accurate mockups (open the .dc.html in a browser)
fixtures/saves/       real .sav files (Dunarr-076, 269, Another-1-2)
```

Two invariants shape everything: **Rust owns canonical state** (the renderer is
a projection patched by events), and **solves never move geometry** — numbers
change, cards don't. Every mutation is one undoable step, with solver
write-backs folded into the causing command's entry.

## Running locally

### Prerequisites

- Rust (stable; developed on 1.94) — https://rustup.rs
- Node 22+ and pnpm 10+ — `npm i -g pnpm`
- `wasm-pack` only if you touch `crates/solver` (the built pkg is committed)
- Linux desktop shell only: WebKitGTK 4.1 dev packages
  (`libwebkit2gtk-4.1-dev libgtk-3-dev` or your distro's equivalent)

### Headless dev flow (recommended — no display or GTK needed)

The `dev-bridge` binary exposes the *identical* command surface as the Tauri
shell over HTTP; the renderer can't tell the difference. State stays canonical
in Rust either way.

```sh
# terminal 1 — the real Rust core on port 8791
cargo run -p app --no-default-features --bin dev-bridge

# terminal 2 — the renderer on port 5173 (proxies /api → 8791)
cd renderer && pnpm install && pnpm dev
```

Open http://localhost:5173. You'll land on the first-run card — place a
factory (N), run the wizard (P), or import a save (S). The plan persists to
`dev-world.ficsit` in the repo root by default; delete the file (and its
`-wal`/`-shm` siblings) for a fresh world.

### Windows: download the exe

Every merge to `main` publishes a portable **FICSIT-Planner.exe** as a GitHub
Release (auto-tagged `v0.1.<build#>`) — grab it from the repo's Releases page
and double-click. Branch pushes upload the same exe as a run artifact instead.
The renderer is embedded in the binary; the only runtime dependency is
Microsoft's WebView2 (preinstalled on Windows 10/11). Pushing a `v*` tag by
hand releases under that exact version. The plan file lives in
`%APPDATA%\dev.ficsit.planner\`.

### The desktop app (from source)

Debug builds point the window at the vite dev server, so run both:

```sh
cd renderer && pnpm dev          # terminal 1
cargo run -p app --features shell   # terminal 2 (needs WebKitGTK on Linux)
```

The shell stores its plan in the OS app-data directory
(`…/dev.ficsit.planner/world.ficsit`).

For a self-contained binary (embedded renderer, no vite), build the renderer
first and use the `prod` feature — plain `--release` is **not** enough, because
Tauri decides dev-vs-production by feature, not by profile:

```sh
cd renderer && pnpm build
cargo build -p app --features prod --release
```

### Environment variables

| Variable | Meaning | Default |
|---|---|---|
| `FICSIT_PLAN` | Plan-file path (dev-bridge only) | `dev-world.ficsit` |
| `FICSIT_BRIDGE_PORT` | dev-bridge port | `8791` |
| `FICSIT_DOCS_JSON` | Path to `<install>/CommunityResources/Docs/Docs.json` — loads the full game catalog (UTF-16LE handled) | bundled fixture |
| `FICSIT_GAME_BUILD` | Build label stored with extracted gamedata | `fixture` |
| `FICSIT_AI_KEY` | Model API key; absent ⇒ `AI OFFLINE` and the heuristic engine feeds the same advisor/chat surfaces | unset |

Without a game install, the bundled fixture (the Modular Frame chain plus
belts, splitters, coal power, and one alternate recipe) keeps every feature
working offline. Imported saves that reference recipes outside the fixture
render as ◆ built factories but report `solve_error` until a real Docs.json is
configured — import is enrichment, never load-bearing.

### A five-minute tour

1. **Map** — N places a factory; click its pin → summary drawer → OPEN
   FACTORY. Claim a node from the node drawer, wire ports → machines → ports
   in the graph, then drag the OUTPUT TARGET slider: T0 projects in italics on
   drag, T1 settles upright on release, and the slider hard-stops at the named
   binding constraint.
2. **Empire** — right-drag pin→pin to draw a route (belt/rail/truck/drone by
   distance-based suggestion, or a ⚡ power line). Rail routes show the full
   math block: round trip × 1.12 terrain, dwell, editable headway, RTT,
   throughput vs demand. TAB opens the audit drawer (saturation, deficits,
   power margins + brownout sim, plan drift).
3. **Wizard** — P, pick an item and rate, SOLVE. The global solver streams its
   log (demand graph → recipes → siting → routing, with power sourcing per
   Addendum A2.4), then hands you a proposal: check/uncheck rows with live
   consequence recompute, then ACCEPT — one undo step, ◇ planned entities
   only. Infeasible returns best-achievable plus one-tap relaxations.
4. **Import** — IMPORT SAVE, pick a `.sav` from `fixtures/saves/`. First
   import writes the built layer (Dunarr-076 → 13 auto-named factories);
   re-imports never write — drift arrives as a reviewable proposal and in the
   PLAN DRIFT tab.
5. **Advisor** — A opens the right-edge panel. Cards fire once per
   newly-armed condition with SAW/RULE provenance; DISMISS mutes that rule for
   good. The CHAT tab answers "power"/"deficits" from live state and turns
   "produce Iron Rod at 30/min" into a proposal through the solver.

Keys: `N` factory · `P` wizard · `A` advisor · `TAB` audit · `F` frame ·
`1/2/4` overlays · `⌘K` search · `⌘Z/⌘⇧Z` undo/redo (solves included) ·
`Enter` dive into selection · `Esc` back/deselect.

## Testing

```sh
# Rust: unit + integration (solver golden cases, session flows, proposals,
# import/drift, advisor gating) — headless, no GUI deps
cargo test --workspace --exclude app && cargo test -p app --no-default-features

# lint + format (CI enforces both)
cargo fmt --all --check
cargo clippy --workspace --exclude app -- -D warnings
cargo clippy -p app --no-default-features -- -D warnings

# renderer typecheck
cd renderer && pnpm typecheck

# end-to-end: every phase exit criterion through the real UI against the real
# core (starts its own dev-bridge on a throwaway plan + vite)
cd renderer && pnpm exec playwright test
# in a container with a preinstalled browser:
#   PW_EXECUTABLE=/path/to/chromium pnpm exec playwright test
```

The e2e suite runs serially against one shared backend: phase specs build on
each other (the phase-2 empire feeds phase-3's deficits, phase-4's import
feeds phase-5's advisor).

## Regenerating artifacts

- **Design tokens** are defined once in `crates/app/src/tokens.rs`:
  `cargo run -p app --no-default-features --bin gen-tokens` rewrites
  `renderer/src/tokens/`. CI fails on drift. No hex value ships outside the
  token system.
- **T0 WASM** (after touching `crates/solver`): `scripts/regen-wasm.sh`
  rebuilds `renderer/src/wasm/pkg` with wasm-pack and stamps the solver
  sources that fed it. CI fails on drift (`scripts/regen-wasm.sh check`).
- **Demo seed** (a small empire with routes, a coal grid, and floors — good
  for screenshots and manual poking): start the bridge on a *fresh* plan plus
  vite, then `cd renderer && node seed.mjs`.

## Data, assets, licensing

- `.sav` parsing uses the community-maintained
  `@etothepii/satisfactory-file-parser` in a Web Worker; unknown/modded
  classes are quarantined and counted, never silently dropped. Parse failure
  degrades to manual entry — no dead ends.
- Map tiles and the community icon pack are **not bundled**: both wait on the
  same licensing review (open question for a human in `DECISIONS.md`). The
  survey grid + diagonal-stripe placeholders are the designed fallback, not an
  error state.
- The bundled world-node snapshot is a hand-assembled subset with plausible
  elevations and two cave nodes; the full community snapshot swap-in is
  backlogged behind the same review.

## Where to read more

- `docs/04-sdd.md` — the authoritative system design (architecture, solver
  tiers and budgets, import pipeline, AI layer, phasing).
- `docs/02-ui-spec.md` + `docs/03-addendum-a.md` — every screen and the
  binding interaction principles (status grammar ◇◈◆, "orange is a verb",
  routes-are-entities, the solver budget contract).
- `design-reference/FICSIT Planner — Foundations.dc.html` — the pixel mocks.
- `DECISIONS.md` — every deviation/extension, one line each, with doc refs.
- `BACKLOG.md` — deferred work, tagged v1.1/v2.

Annotated tags `phase-1` … `phase-5` mark each exit-criterion commit (local
only — this integration can't push tags; re-tag after merge).
