# BACKLOG

Everything tempting that isn't Phase 1. Tags: v1 (later phase of this release), v1.1, v2.

- v1: Full community world-node snapshot (all nodes, all purities, geysers) replacing the Phase 1 subset — pending tile/data licensing review.
- v1: Community map tile set (3 zoom levels) once licensing is verified (SDD §7).
- ~~v1: Inter-factory routes + empire recompute + audit drawer~~ — DONE (Phase 2).
- ~~v1: Circuits, generator factories~~ — DONE (Phase 2); priority switches still open (needs persisted Circuit entities).
- ~~v1: Proposal system, global solver, wizard, T2 mini-proposals~~ — DONE (Phase 3).
- ~~v1: Rail/truck/drone inspectors + save import~~ — DONE (Phase 4).
- ~~v1: AI layer — advisor gate, chat, proposal_intent, style guides~~ — DONE (Phase 5, offline-honest; model relay scaffolded).
- v1.1: Batteries/storage in circuit math (`⚡ +BUFFER` slot reserved, A2.2).
- v2: Signal-block rail capacity modeling (A3.1); direct .pak icon/tile extraction (SDD §7).
- v1: Belt tier picker on connection drop (currently defaults Mk.1, editable via inspector/edge select) — UI spec 4c.
- ~~v1: Right-drag route drawing from pins on the map~~ — DONE (Phase 2); drawing from *nodes* still open.
- v1: "Plan a move" flow for built factory pins (built layer arrives with import).
- v1.1: True shared-element map↔factory zoom morph (currently scale+fade).
- v1.1: Node index numbers on map labels (`FE PURE #08` style) once the full node snapshot lands.
- v1.1: Belt corridor bundling (shared-lane routing when many belts run parallel) and manual waypoint pins on belts.
- v2: Machine footprint dimensions from pak extraction replacing the community table in `footprints.ts`.
- v1: Supplemental generator fluids (water for coal/nuclear) — lands with pipes.
- v1.1: Route waypoint editing (paths are multi-point in the model; UI draws straight pin-to-pin).
- v1.1: Persisted Circuit entities: user naming, breakers, priority switches (grids are derived-only today).
- v1: Pipe head-lift math from route climb (z groundwork landed; needs pipes).
- v1: Rail grade warnings from per-segment slope (z groundwork landed; needs Phase 4 rail).
- v1.1: Elevation from a licensed heightmap (auto-fill pin z + climb along interior waypoints) — replaces planner-entered z.
- v1.1: True cross-item MILP recipe selection (per-item scoring today; matters once alternates create trade-offs).
- v1.1: Multi-site decomposition in the global solver (one integrated site per solve today); EXPAND-vs-GREENFIELD preference currently only expands generators.
- v1.1: Proposal detail popover (BEFORE→AFTER table, mock 3b) + EDIT VALUES on items; D diff key.
- v1.1: Unlock/progression model (milestones + hard drives) — alternates render locked until then.
- v1.1: Multi-item rail manifests (needs multi-port route binding; math block already sums the manifest).
- v1.1: Belt-bridged cluster merging + belt/rail/power route reconstruction from save connection components (counts imported today).
- v1.1: Truck fuel + drone batteries injected as solver demand (rendered as line items today).
- v2: Signal-block rail capacity modeling (fixed 15% headway today, A3.1).
- v1.1: Streaming model relay (SSE) behind FICSIT_AI_KEY + OS-keychain storage; advisor prose pass over armed events within the visible budget.
- v1.1: Image→style-guide vision call (surface + entity are live; needs the model relay).
- v1.1: An empty upstream factory feeds routes unconstrained (no groups → no supply entry); tighten to zero-supply once boundary-only factories have a defined meaning.
- v1.1: @-entity references in the chat composer; chat history persistence.
