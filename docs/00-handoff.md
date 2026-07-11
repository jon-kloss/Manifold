# HANDOFF — FICSIT Planner Implementation

You are Claude (Fable 5) running in Claude Code. You are the implementing engineer for **FICSIT Planner**, a desktop factory/logistics planner for Satisfactory. The product is fully designed and architected; your mandate is to build it, starting with Phase 1. You have latitude on implementation details and none on the invariants.

## Read order and authority

Read all four docs completely before writing any code. When they conflict:

1. `docs/04-sdd.md` — **Software Design Document.** Authoritative on architecture, data model, solver design, IPC, persistence, testing, and the phase plan (§12). This is your primary contract.
2. `docs/03-addendum-a.md` — gap resolutions (responsive system, power, trains, solver budget tiers). Normative; amends the UI spec.
3. `docs/02-ui-spec.md` — the design handoff. Normative on tokens, status grammar, every screen, every interaction. Pixel-fidelity expectations are stated inside it. The addendum overrides it where they overlap (notably: the 1920×1080 floor is replaced by A1).
4. `docs/01-design-brief.md` — the original product brief. Context only; superseded where the others are specific.

`design-reference/FICSIT Planner - Foundations.dc.html` (open in a browser; `support.js` must stay beside it) contains the 30 pixel-accurate mockups (`1a`–`7d`) the UI spec references. Consult it whenever a spec sentence is ambiguous — the mock is the tiebreaker on visuals. Diagonal-stripe blocks are runtime-asset placeholders (icons, map imagery); reproduce the placeholder treatment, never invent icon art.

`fixtures/saves/` contains real .sav test files for the Phase 4 importer (see `fixtures/README.md` for provenance and what each exercises). They are not needed until Phase 4 — do not let them pull import work forward.

## Your mandate: Phase 1

Per SDD §12. Scope: Tauri 2 shell with custom titlebar, design-token system, responsive layout system (Addendum A1), map home (Leaflet CRS.Simple, world nodes from a bundled static snapshot, manual factory placement), factory graph view (React Flow), T0 (WASM) + T1 (Rust LP) solvers with the budget contract (A4), gamedata pipeline from Docs.json, SQLite plan file with undo log.

**Exit criterion:** a user can plan the Modular Frame factory end-to-end, fully offline — place a factory on the map, build its production chain in the graph, drag the target rate and watch the chain re-solve live with belt saturation coloring, and reopen the app to find everything persisted with working undo.

Later-phase surfaces (proposals, advisor, wizard, import, trains, power planning) must not be stubbed into the UI. Build the *data model* to the full SDD §3 shape from the start — entities carry `status` and `created_by` from day one even though Phase 1 only creates Planned entities manually.

## Non-negotiable invariants (SDD §3.1 + UI-spec principles)

- Rust owns canonical state; the renderer is a projection patched by events. No state forking.
- Solves never add/move/remove cards or pins — numbers change, geometry doesn't.
- Every mutation is undoable; solve-induced changes fold into the causing command's undo entry.
- The status grammar (◇ ◈ ◆, stroke + fill + glyph) and the color rules ("orange is a verb", blueprint blue = planned only, Flow colors = load/status only) are law from the first component. Do not ship a single hex value outside the token system.
- All numbers render in JetBrains Mono with tabular-nums; projected values are italic.
- No dead ends: infeasible solves return the binding constraint and the UI hard-stops, never errors.

## Working agreements

- **Setup first:** scaffold the Cargo workspace (`planner-core`, `solver`, `gamedata`, `persist`, `app`) and the React app per SDD §2 before feature work. Get the WASM T0 build working early — if wasm-pack integration burns more than a focused effort, take the SDD's sanctioned TS fallback (identical interface, golden parity tests against the Rust implementation) and leave a note in `DECISIONS.md`.
- **Keep a `DECISIONS.md`** at repo root: every place you exercised judgment beyond the docs, one line each — what, why, which doc section it touches. This is the review surface for the human.
- **Verify as you go.** Solver golden tests (the Modular Frame chain is fully worked in SDD §11 spirit: at target T, ore = 24T/min, rods = 10.5T, screws = 18T, plates = 9T, RIP = 1.5T — use these as the first golden case). Run the app and interact with it; screenshot the map and graph against mocks `2a` and `4a` before calling a surface done.
- **Commit discipline:** small commits, imperative messages, one concern each. Tag the Phase 1 exit-criterion commit.
- **Scope control:** anything tempting that isn't Phase 1 goes into `BACKLOG.md` tagged v1/v1.1/v2 — the SDD risk table names scope as the project's top risk for a reason.
- **Stop and ask** (leave a question in `DECISIONS.md` and pause that thread) only for: license questions on bundled map tiles/community assets, deviations from the token system, or any urge to change an invariant. Everything else: decide, note it, proceed.

## Suggested first session

1. Read all docs + skim the mock HTML. 2. Scaffold workspace + CI (fmt, clippy, test, tsc). 3. Tokens as a single source (Rust-side constants generated to CSS custom properties + TS module). 4. planner-core entities + SQLite schema + undo log with tests. 5. Tauri shell: custom titlebar + status bar + the A1 layout system with resize behavior. Then map, then graph, then solvers.

Build it like it's going on your portfolio next to Wire — because it is.
