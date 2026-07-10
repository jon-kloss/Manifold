# FICSIT Planner

A desktop factory & logistics planner for Satisfactory. Tauri 2 shell, React 19
renderer, Rust core owning canonical state, solvers, and persistence. **The map
is the source of truth** — factories are placed entities claiming real resource
nodes; plans are living blueprints that re-solve as you drag.

Phase 1 (this build): map home + factory graph + T0/T1 solvers + gamedata
pipeline + single-file plan persistence with full undo. See `docs/` for the
complete design contract (`04-sdd.md` is authoritative) and `DECISIONS.md` for
every judgment call made during implementation.

## Layout

```
crates/planner-core   domain model, canonical state, commands, undo log
crates/solver         T0 ratio propagation + T1 LP (good_lp/microlp)
crates/solver-wasm    wasm-pack wrapper over T0 for renderer drag frames
crates/gamedata       Docs.json parser → gamedata.sqlite; world-node snapshot
crates/persist        world.ficsit plan file (SQLite, WAL, undo journal)
crates/app            Tauri 2 shell + headless dev-bridge + token generator
renderer/             React 19 + Zustand + Leaflet + React Flow
docs/                 the handoff: SDD, UI spec, Addendum A, brief
design-reference/     pixel-accurate mockups (open the .dc.html in a browser)
fixtures/saves/       real .sav files for the Phase 4 importer
```

## Development

```sh
# Rust core: build + test (no GUI deps needed)
cargo test --workspace --exclude app && cargo test -p app --no-default-features

# T0 solver → WASM (once, or after touching crates/solver)
cd crates/solver-wasm && wasm-pack build --target web --out-dir ../../renderer/src/wasm/pkg --release

# Renderer against the real Rust core, headless (no display required)
cargo run -p app --no-default-features --bin dev-bridge   # port 8791
cd renderer && pnpm install && pnpm dev                   # port 5173, proxies /api

# The desktop app (needs WebKitGTK on Linux)
cargo build -p app --features shell

# End-to-end suite (drives the real core through the UI)
cd renderer && npx playwright test
```

Design tokens are defined once in `crates/app/src/tokens.rs`; regenerate the
CSS/TS with `cargo run -p app --no-default-features --bin gen-tokens`. CI fails
if the generated files drift. No hex value ships outside the token system.

Game data: without a game install the bundled fixture (Modular Frame chain)
keeps everything working offline. Point `FICSIT_DOCS_JSON` at
`<install>/CommunityResources/Docs/Docs.json` to load the full game.
