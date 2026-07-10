# Addendum A: Gap Resolutions — FICSIT Planner Design Handoff

Extends `README.md` (the handoff spec). Same tokens, status grammar, and interaction principles apply. Every spec below is normative. Sections: A1 responsive degradation · A2 power planning · A3 train/logistics planning · A4 solver budget contract. Two principles are added to the binding list (see A5).

---

## A1. Responsive Degradation (replaces the 1920×1080 fixed floor)

**New floor: 1366×768 hard minimum. 1920×1080 is the reference density**, not a requirement. The canvas (map or graph) is the only flex element; panels never fluid-squish — each panel has exactly two docked widths (full / compact) plus an overlay conversion. Typography never scales with window size.

| Range | Behavior |
|---|---|
| ≥ 1920 | Layout exactly as specced. |
| 1600–1919 | Compact panel widths: inspector 360→320, advisor 340→300, proposal panel 470→420, summary drawer 380→340. Legend collapsed by default. Recipe strip 640→560. |
| 1366–1599 | **Overlay mode:** inspector, advisor, and summary drawer slide *over* the canvas (scrim 20%) instead of docking; audit drawer opens at 40% height; recipe strip 520 with horizontal scroll; status-bar counts collapse into one `⋯` chip with popover; titlebar breadcrumb middle-truncates. |
| < 1366 | Refuse gracefully: centered card "FICSIT Planner needs at least 1366×768" + current size in mono. Never render broken. |

Rules: on window resize, canvas re-centers on the prior focal point (zoom/position never lost — Principle 1 applies to resize too). Docked→overlay conversion animates 200ms (drawer curve); reduced-motion = instant. All dimensions are CSS px; verify at 125% and 150% OS scaling — the 1366–1599 overlay mode is what a 1920 laptop at 125% actually gets, so it is a first-class layout, not a fallback. Keyboard reach is unchanged in every mode; nothing becomes hover-only.

---

## A2. Power Planning (power is production, not telemetry)

The insight that keeps this cheap: **a power plant is a factory.** Fuel chains are recipes; generators are machine groups whose output is MW. Generation planning therefore reuses the entire factory-view surface, solver, and proposal system. What's genuinely new is grid topology and priority.

### A2.1 Circuits
The POWER overlay is upgraded from read-only to an editable layer. Each connected grid is a **circuit**: tinted region hull on the map + a circuit chip `GRID A · 4.2/6.0 GW` (mono, flow-colored by margin: OK ≥ 20% headroom, WARN 5–20%, CRIT < 5%). Power lines are routes of type `power` — single 2px line, but the label chip shows **circuit margin, not link load** (power is a bus, not a belt). Right-drag between factories/pylons draws a planned power line (blueprint-dashed, per the grammar). Merging two circuits by connecting them shows a confirm popover with the combined margin *before* commit.

### A2.2 Generator factories
A generator machine-group card is a standard card whose footer reads `IN coal 240/min + water 540/min · OUT 16 × 75 MW`. The factory-view target slider for a generation factory is denominated in **MW**; the local solver back-solves the fuel chain exactly as it does items. Geothermal = a claimable node (map grammar already covers it; purity ring applies). Batteries/storage: **v1.1**, note in backlog — the circuit chip reserves a `⚡ +BUFFER` slot in its popover for it.

### A2.3 Priority switches
A switch is a map entity: **18px square pin** (square = infrastructure; diamond = factory — this distinction is now grammar), status glyphs apply, chip `P3 · SHEDS AT 5.8 GW`. The audit POWER tab gains two columns: `PRIORITY` and `SHEDS AT` (mono GW value at which this group drops given current demand trajectory), plus one synthetic first row per circuit: `BROWNOUT SIM — next shed: P4 LIGHTING @ +0.4 GW growth` with `TRACE` action.

### A2.4 Proposals must source their power
The Δ POWER impact cell becomes load-bearing. When a proposal's demand exceeds the target circuit's margin, the cell renders WARN/CRIT **and the global solver auto-appends a POWER group to the change list** — either `Δ EXPAND COASTAL COAL +4 GENERATORS` (with its fuel-chain sub-items) or `+ NEW SITE — FUEL GEN` — solved with the same LP as any input. A proposal that browns out the grid is infeasible-by-default; the wizard's power-margin-cap constraint is the knob. Users can exclude the POWER group like any row; the live consequence recompute then shows the CRIT margin they're accepting (amber strip: "Excluding power leaves GRID A at −0.6 GW — plan will show a deficit.").

---

## A3. Train & Logistics Planning (routes are entities)

Routes get the same first-class treatment as factories: click any route → **Route Inspector** (380px drawer, mirror of the factory summary drawer, status grammar applies — a planned route is ◇ with italic projections).

### A3.1 Rail route inspector
Header: route name, endpoints (entity chips, clickable), status chip, length `3.4 km` (planned = path length × 1.12 terrain factor, italic). Sections:

- **MANIFEST** — a rail route carries multiple items: table of item icon + name + assigned /min + share bar. Sum drives the math below.
- **CONSISTS** — rows in mono: `2× LOCO + 6× FREIGHT` with `+ / −` steppers on cars and consist count.
- **THE MATH BLOCK** (this is the product; render it, don't hide it):
  ```
  ROUND TRIP   2×3.4km @ ~90km/h        4:32
  LOAD/UNLOAD  2 stations × 0:25        0:50
  HEADWAY      fixed penalty 15%        0:48   [edit]
  RTT                                   6:10
  THROUGHPUT   6 cars × 32 stacks …  288/min
  DEMAND                              480/min  ⚠ CRIT
  ```
  Changed values settle-flash; the throughput vs demand line carries the flow color and drives the route's encoding on the map.
- **SUGGESTION ROW** (only when short): `+1 CONSIST → 576/min ✓ · +1 LOCO +6 CARS` — one tap applies to a ◇ route directly; on a ◆ built route it becomes a ◇ delta (existing pattern).
- **STATIONS** — per-station platform count and dwell time (feeds the math block).

Signal-block capacity modeling is **v2**; v1 uses the fixed, editable 15% headway penalty, always visible in the math block — honest approximation over hidden precision.

### A3.2 Trucks and drones
Same inspector, simpler math blocks: trucks = path time + fuel chain line-item (fuel is an input the solver sources); drones = battery round-trip limit + batteries/min consumed (likewise solver-sourced). One params table in the data model covers all three; the inspector renders whichever rows apply.

### A3.3 Wizard integration
The global solver's ROUTING phase now emits transport payloads: proposal route rows read `⟶ RAIL — COASTAL STEEL ⟶ MOTOR WORKS · 2 CONSISTS · proj 62% · 3.4 km`. The solver picks transport type by distance/rate thresholds (belt < 800m, rail ≥ 800m or ≥ 480/min, drone for < 60/min over ≥ 1.5km) — thresholds live in the constraints grid, step 1.

---

## A4. Solver Budget Contract (what happens when 5ms is a lie)

Three solve tiers, each with an explicit budget and a defined miss behavior. The solve-time readout (`LAST 2.8ms ✓`) is always present and always honest.

| Tier | Trigger | What it solves | Budget | Sync? |
|---|---|---|---|---|
| T0 ratio | slider drag, clock change | pure ratio propagation, fixed structure | 5ms | yes |
| T1 LP | slider release, tier/target commit | LP, **fixed recipe set** | 50ms | async |
| T2 optimize | explicit `OPTIMIZE RECIPES` action | LP with alt-recipe selection | 2s | async |

- **During drag**, T0 numbers render *italic* (projected — the grammar already means this). On release, T1 lands and numbers settle-flash to upright. If T1 returns within 50ms of drag-end this is imperceptible; the italic tier is the honesty valve, not a loading state.
- **T1 miss behavior:** three consecutive solves > 50ms → solve-time chip goes amber (`SOLVE 84ms`) and the slider switches from solve-on-drag to solve-on-release with a `LIVE → ON RELEASE` chip. Never silently degrade; never block the slider.
- **T2 never runs implicitly.** Recipe selection changes cards — and Principle 5 says solves never change geometry. Therefore alt-recipe optimization is an explicit button and its result arrives as a **mini-proposal** (existing review pattern, scoped to one factory: swap rows with BEFORE→AFTER). This closes the loophole cleanly: on-drag solves structurally *cannot* violate Principle 5.
- **Empire recompute** (audit LIVE, route/power projections): incremental dirty-graph propagation on the Rust side, 200ms budget, off the UI thread; only affected audit rows shimmer `RE-AUDITING…`. Full recompute (save import, proposal accept) shows the status-bar solver chip.

---

## A5. Additions to the binding principles list

9. **Power is production.** Generation is planned with the same factory surface and solver as items; proposals must source the power they consume.
10. **Routes are entities.** Every route has an inspector, a math block you can read, and the ◇◈◆ grammar; throughput is computed, never asserted.

## Phasing amendment

Phase 1 (map + graph + T0/T1 solver + tokens) is unchanged. A1 lands in Phase 1 (layout system is foundation, not polish). A4 lands with the first solver. A2 circuits + generator factories land in Phase 2 with the dependency graph; priority switches with the audit surface. A3 rail inspector lands with the trains phase; trucks/drones follow. T2 optimize and the mini-proposal land with the proposal system.
