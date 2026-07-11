# FICSIT Planner — Software Design Document (SDD)

**Version:** 1.0 · **Status:** Draft for review
**Companion docs:** Design Handoff README (UI/UX spec), Addendum A (gap resolutions). Where this SDD and the design docs conflict, the design docs win on *what*; this SDD wins on *how*.

---

## 1. System Overview

A desktop factory/logistics planner for Satisfactory. Tauri 2.x shell; React 19 + TypeScript renderer; Rust backend owning canonical state, solvers, persistence, and AI orchestration. The map is the source of truth: all entities are spatially placed, and every mutation flows through one of two paths — direct plan edits (local solve) or reviewable proposals (global solve, advisor, chat, save re-import).

```
┌────────────────────────── Tauri Window ──────────────────────────┐
│  React renderer                                                  │
│  ┌─────────┐ ┌───────────┐ ┌──────────┐ ┌─────────────────────┐  │
│  │ Map     │ │ Factory   │ │ Proposal │ │ Advisor / Chat      │  │
│  │ Leaflet │ │ ReactFlow │ │ Review   │ │                     │  │
│  └─────────┘ └───────────┘ └──────────┘ └─────────────────────┘  │
│  Zustand projected store · T0 solver (WASM) · Save-parse Worker  │
└───────────────────────────── IPC (commands + events) ────────────┘
┌──────────────────────────── Rust core ───────────────────────────┐
│ planner-core (domain, canonical state, undo log)                 │
│ solver (T1 LP, T2 optimize, empire recompute)   gamedata (recipes│
│ advisor (gate rules, budget, Anthropic client)   + icons + map)  │
│ importer (clustering, diff-to-proposal)          persist (SQLite)│
└───────────────────────────────────────────────────────────────────┘
```

## 2. Technology Decisions

| Concern | Decision | Rationale |
|---|---|---|
| Shell | Tauri 2.x | Wire-stack parity; small binaries; Rust backend for solver perf |
| UI | React 19 + TypeScript, Zustand, React Flow (graph) | React Flow is the decided graph editor; Zustand for a thin projected store |
| Map | Leaflet + `CRS.Simple` over extracted/community map tiles; custom canvas overlay layer for routes, pins, circuits | Proven at Satisfactory scale by SCIM; canvas overlay avoids thousands of DOM markers |
| Solver LP | `good_lp` with the pure-Rust `microlp`/`clarabel` backend | No C toolchain dependency → trivial cross-platform bundling |
| T0 solver in renderer | `solver-core` crate compiled to WASM via wasm-pack | Single source of truth for ratio math; no IPC on drag frames. Fallback if WASM pipeline stalls: hand-ported TS T0 with golden-test parity against Rust |
| Save parsing | `@etothepii/satisfactory-file-parser` in a **Web Worker in the renderer** (browser-compatible per upstream), streaming mode, `throwErrors: false` | Validated against real fixtures (Dunarr-076: 21k buildables incl. mod objects parsed cleanly); avoids a Node sidecar entirely; quarantines unknown objects instead of failing |
| Persistence | SQLite via `rusqlite` — one `.ficsit` plan file per world | Single-file portability; tables double as undo journal and proposal archive |
| AI | Anthropic Messages API from Rust (`reqwest`); key in OS keychain (`keyring` crate) | Backend-only key handling; renderer never sees it |
| Fonts/assets | Rajdhani, Barlow, JetBrains Mono bundled; game assets never shipped (§7) | License posture per design spec |

## 3. Domain Model (planner-core)

All entities carry `id: Ulid`, `status: Planned | UnderConstruction | Built`, `createdBy: Manual | Proposal(id) | Import(id)`.

```rust
Factory        { name, position: MapPos, region, node_claims: Vec<NodeClaimId>,
                 groups: Vec<MachineGroupId>, ports: Vec<PortId>, style_guide: Option<StyleGuideId> }
MachineGroup   { factory, machine: MachineClass, recipe: RecipeId, count: u32,
                 clock: f32, somersloops: u8, planned_delta: Option<DeltaRef> }
Port           { factory, direction: In|Out, item: ItemId, rate: RatePerMin, bound_route: Option<RouteId> }
Route          { kind: Belt(Tier) | Pipe(Tier) | Rail(RailSpec) | Truck(TruckSpec)
                       | Drone(DroneSpec) | Power, path: Polyline, endpoints: (PortRef, PortRef),
                 manifest: Vec<(ItemId, RatePerMin)> }
RailSpec       { consists: u8, locos: u8, cars: u8, stations: Vec<StationSpec>, headway_penalty: f32 /*0.15*/ }
NodeClaim      { node: WorldNodeId, factory, extractor: MachineClass, clock: f32 }
Circuit        { name, members: Vec<FactoryId>, switches: Vec<SwitchId> }   // derived + persisted
Switch         { position, circuit_a, circuit_b, priority: u8 }
Proposal       { source: GlobalSolver|Advisor|Chat|SaveReimport, goal, snapshot_time, input_hash,
                 items: Vec<ProposalItem { kind: Create|Modify|Claim|RouteAdd, included: bool,
                                            payload, consequences }>,
                 state: Draft|Reviewing|Accepted|Rejected, stale: bool }
```

**Static world data** (resource node positions/purities, biomes) ships as a versioned JSON snapshot derived from community map data — saves don't contain node metadata, so this is bundled, not extracted.

**Derived state** (never persisted, always recomputed): route loads, circuit margins, deficits, plan-vs-built drift, rail throughput. Computed by the empire recompute pass (§5.4).

### 3.1 Invariants (enforced in planner-core, not the UI)
1. Built entities are immutable except via import; edits to built entities materialize as `planned_delta`.
2. Proposal accept = one transaction, one undo entry, creates Planned entities only.
3. A `NodeClaim` conflict (two claims on one node whose combined rate exceeds extraction ceiling) is representable — it renders as CRIT, it is never silently prevented (users may intend it temporarily).
4. Route manifest rates must equal bound port rates (solver maintains; manual edits re-solve).

## 4. State Architecture & IPC

**Rust owns canonical state.** The renderer holds a *projected store* (Zustand) hydrated at load and patched via Tauri events (`state://patch`, JSON-Patch batches). All mutations are Tauri commands returning `Result<PatchBatch, DomainError>`:

```
plan.edit(op)            // T0/T1 path: slider commit, recipe swap, tier change, drag-move
plan.undo() / redo()
proposal.create(goal)    // global solver entry
proposal.toggle_item / edit_item / accept / reject / resolve_stale
route.edit(spec)         // consists, manifests, dwell
import.run(parsed_json)  // from the parse worker → diff → proposal
advisor.query(scope)     // chat; advisor feed pushes via events
assets.extract(path)     // onboarding
```

**Optimistic drag:** during slider drag the renderer runs WASM T0 locally and renders italic projected values; on release it issues `plan.edit`, Rust runs T1, and the authoritative patch settle-flashes. If the T1 result differs from T0's projection beyond epsilon, the flash makes the correction visible — honesty by construction.

**Undo:** command-sourced. Each accepted mutation appends `(inverse_patch, forward_patch)` to the `undo_log` table. Solve-induced value changes are folded into the causing command's entry (⌘Z undoes the edit *and* its solve, per the design spec).

## 5. Solver Subsystem

### 5.1 T0 — ratio propagation (WASM, <5ms)
Fixed structure, fixed recipes. Topological walk of the factory's group DAG scaling counts/clocks/rates linearly from the changed target. Pure function: `(FactorySnapshot, Edit) -> ProjectedRates`.

### 5.2 T1 — local LP (Rust, 50ms budget)
Per-factory LP, fixed recipe set: variables = group clocks/counts (integer counts relaxed, then rounded with clock redistribution); constraints = port capacities (belt tiers), input ceilings, node extraction rates; objective = meet target, minimize machines then power. Infeasible → return the binding constraint (the UI hard-stops the slider at the ceiling and names it). Emits per-solve timing; three consecutive >50ms results flip the factory to solve-on-release mode (renderer behavior, driven by a flag in the patch).

### 5.3 T2 — recipe optimization (explicit, 2s budget)
Same LP with recipe-selection binaries over unlocked alts (MILP via branch-and-bound on the small per-factory recipe set; factory-scoped so dimensionality stays low). Output is **never applied**: it is diffed against current structure into a factory-scoped mini-proposal.

### 5.4 Empire recompute (200ms budget, dedicated thread)
Incremental dirty-propagation over the inter-factory graph: port rate changes → route loads → downstream input ceilings → circuit demand → rail throughput vs manifest → audit rows. Dirty-set seeded by each patch; full recompute only on import/accept. Publishes `audit://patch` events; renderer shimmers affected rows only.

### 5.5 Global solver (async, cancellable, 5–15s typical)
Pipeline of phases, each streaming log lines (`solver://log`) and progress:
1. **Demand graph** — expand goal items to raw resources given constraints and existing surplus (surplus-first toggle consumes existing overproduction before proposing new).
2. **Recipe selection** — MILP over unlocked (+optionally locked-suggest) alts, empire-scoped but pruned to the demand cone.
3. **Siting** — deterministic scoring over candidate sites: unclaimed node distance-weighted availability, route distance to consumers, expand-vs-greenfield preference, node budget/purity floor. Top-k per new factory retained for the review popover.
4. **Routing** — transport type by thresholds (belt <800m, rail ≥800m or ≥480/min, drone <60/min over ≥1.5km; user-tunable), consist counts from the rail math (§6), power sourcing pass (§ A2.4): demand beyond circuit margin appends generation expansion items solved through phases 1–3 recursively (depth 1).
Output: `Proposal` with per-item consequences. Cancellation is cooperative between phases and inside MILP node limits.

## 6. Route Math (shared by inspector, recompute, and global solver)

```
rail:  rtt = 2·len·terrain(1.12 planned)/avg_speed + Σ dwell + headway·travel
       throughput/min = consists · cars · stacks_per_car · stack_size / rtt_min
truck: analogous + fuel_rate item demand injected into solver inputs
drone: trips/min bounded by battery round-trip; batteries/min injected as demand
power: circuit margin = Σ generator MW − Σ demand MW (per circuit union-find over Power routes)
```
One `TransportParams` table drives all kinds; the Route Inspector renders whichever rows apply. All planned-route figures flagged projected (italic downstream).

## 7. Game Data & Asset Pipeline

- **Recipes/items/machines:** parse `Docs.json` (community-documented, ships with the game install) at onboarding → normalized `gamedata.sqlite` keyed by game build version. Re-parse when install build changes (watch header on app start). This path is low-risk and fully offline.
- **Icons:** v1 downloads a pinned community icon pack (hash-verified) mapped by class name; direct `.pak` extraction (repak + uasset texture decode) is **v2 stretch** behind the same `IconProvider` trait. UI never blocks on icons (diagonal-stripe placeholder chip until resolved).
- **Map tiles:** bundled community-derived tile set at 3 zoom levels (licensing verified before ship); in-game map capture extraction is v2 alongside pak work.
- Provider abstraction: `trait AssetSource { docs(), icon(class), tiles(z,x,y) }` with Install / CommunityPack / Placeholder implementations, chosen at onboarding, swappable in settings.

## 8. Save Import

1. Renderer Web Worker streams the `.sav` through the etothepii parser (`throwErrors:false`); unknown/mod objects land in a quarantine list (count surfaced in the preview per design spec).
2. Worker reduces the raw object soup to a compact `ImportSnapshot`: machines (class, recipe, clock, position), belts/pipes with endpoints, rail tracks/stations/consists, power lines, extractors→node bindings.
3. Rust `importer` clusters machines into logical factories: DBSCAN on XY (eps ≈ 120m, tuned on Dunarr fixtures), then merge clusters bridged by short belts; auto-names by dominant output (`IRON WORKS 1`), all names editable.
4. First import writes the Built layer directly (onboarding contract). **Re-imports never write:** snapshot is diffed against the current Built layer → `Proposal { source: SaveReimport }` with Create/Modify items (drift), reviewed like any proposal.
5. Version tolerance: parser handles U8→1.2 fixtures today; on parse failure the import step degrades to "skip — manual entry" (no dead ends). Fixture suite: `Dunarr-076` (modded, rail-heavy), `269.sav` (clean 1.0), `Another-1-2.sav` (newest format, near-empty).

## 9. AI Layer

- **Client:** Rust, streaming SSE; model configurable; key in OS keychain; renderer receives only streamed text/events. Offline/no-key → `AI OFFLINE` chip; heuristic engine (below) keeps feeding the same feed.
- **Context serializer:** `snapshot(scope: Empire|Factory|Selection) -> CompactState` — dense JSON (ids, names, rates, deficits, claims, phase/milestone, unlocked alts), size shown in the chat context bar; the exact payload is user-viewable (design requirement). Target <30KB empire snapshots via aggregation (per-factory rollups, top-N deficits).
- **Advisor pipeline:** heuristic rules (Rust, pure functions over derived state: `NewDeficit`, `NodeConflict`, `Saturation>75`, `PowerSwing`, `DriftDetected`) → 30s debounce → hourly budget check → single model call contextualizing *only the armed events* → feed cards with `SAW/RULE` provenance persisted. Dismiss mutes per-rule (persisted).
- **Structured outputs:** advisor and chat may return `proposal_intent` blocks (tool-use schema); Rust materializes them through the global-solver validation path so AI-originated proposals carry real consequences, not model arithmetic. The model never mutates state — it can only *draft goals* that the solver solves.
- **Image→style guide:** image + unlocked-parts list → vision call → typed `StyleGuide { palette[], massing, techniques[], sequence[], materials[] }` persisted and linkable to factories.

## 10. Persistence & Files

`world.ficsit` (SQLite): `entities`, `routes`, `proposals`, `proposal_items`, `undo_log`, `advisor_cards`, `mutes`, `style_guides`, `meta` (schema version, game build, window state). WAL mode; autosave on transaction commit (there is no unsaved state); rolling `.bak` on open. `gamedata.sqlite` and icon cache live in app-data, keyed by game build. Export/import of the plan file is trivial by design (single file — community sharing later).

## 11. Testing Strategy

- **Solver:** golden cases (hand-computed chains incl. Modular Frame, fuel loops with byproducts), property tests (mass balance: Σin = Σout ± sinks), T0-WASM vs T1 fixed-point parity within epsilon.
- **Import:** fixture suite from §8.5 asserted on cluster counts, machine totals (Dunarr-076: 21,408 buildables), quarantine behavior on mod objects.
- **Recompute:** dirty-propagation equivalence vs full recompute on randomized edit sequences.
- **UI:** Playwright against the Tauri dev build for the three invariant-critical flows: proposal partial-accept live recompute, undo of accept, drag→release settle.

## 12. Phasing (build order)

| Phase | Scope | Exit criterion |
|---|---|---|
| 1 | Shell, tokens, responsive system (A1), map (nodes/pins/manual factories), factory graph, T0+T1, gamedata from Docs.json, plan file + undo | Plan the Modular Frame factory end-to-end, offline |
| 2 | Inter-factory routes + empire recompute, audit drawer, circuits + generator factories, status grammar everywhere, planned/built dual state | Empire of 5 factories with live saturation + power margins |
| 3 | Proposal system + global solver + wizard, priority switches, T2 mini-proposals | "Plan a supply chain" produces a reviewable, partially-acceptable proposal |
| 4 | Rail/truck/drone inspectors + math, save import (worker + clustering + re-import diff) | Import Dunarr-076; drift renders in DIFF |
| 5 | AI layer (advisor gate, chat, proposal_intent, style guides), onboarding polish, community icon pack pipeline | Ambient advisor survives a week of dogfooding without feeling naggy |

## 13. Risks

| Risk | Mitigation |
|---|---|
| Save format churn on game patches | Parser is upstream-maintained + quarantine mode; import is enrichment, never load-bearing (Principle 6) |
| WASM T0 build friction | TS fallback with golden parity tests; interface identical |
| MILP blowups on large empires | Demand-cone pruning, node limits, per-phase cancellation, "best achievable + relaxations" contract |
| Icon/tile licensing | Community pack license audit before bundling; placeholder grammar means the app is never blocked on assets |
| Leaflet perf at 200+ factories / 1k routes | Canvas overlay layer from day one; virtualized labels; perf budget test in CI at 5× target scale |
| Scope (three products in a trenchcoat) | Phasing table is the contract; every design idea tags v1/v1.1/v2 before entering the plan |
