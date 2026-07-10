# Handoff: FICSIT Planner — Factory & Logistics Planner UI

## Overview
Complete UI/UX design system and screen-by-screen specification for **FICSIT Planner**, a desktop factory/logistics planner for Satisfactory (Tauri + React + Rust). Core positioning: **the map is the source of truth** — factories are placed entities claiming real resource nodes, connected by routes with real distances; power is a second network on the same map. Two solvers (instant local solve, wizard-driven global solve returning reviewable proposals), a dual planned/built state model, and a strictly-additive AI layer (ambient advisor, chat, image→style-guide).

## About the Design Files
The bundled file `FICSIT Planner — Foundations.dc.html` is a **design reference created in HTML** — a spec document with pixel-accurate mockups, not production code. Open it in a browser; it is a pannable canvas of seven "turns" (sections), each design stamped with a stable id (`1a`…`7d`) referenced throughout this README. **The task is to recreate these designs in the target codebase** (React inside Tauri; React Flow for the factory graph was already decided) using its established patterns — do not ship the HTML.

## Fidelity
**High-fidelity.** Colors, typography, spacing, chip/border/pattern treatments and copy are final and should be recreated pixel-perfectly. Two deliberate exceptions, marked with diagonal-stripe placeholders in the mocks: (1) all item/building/machine **icons** and (2) all **world-map imagery** — these are extracted at runtime from the user's game install (see Assets). Never redraw or recolor them.

---

## Design Tokens

### Color — Steel (surfaces, cool near-blacks; the only surfaces, no gradients on chrome)
| Token | Hex | Use |
|---|---|---|
| steel-950 | `#0E1012` | app/canvas background |
| steel-900 | `#15181B` | panels, drawers |
| steel-800 | `#1C2024` | cards, chips |
| steel-700 | `#262B30` | raised/hover, hairline dividers |
| steel-600 | `#333A41` | default border |
| steel-500 | `#49525B` | strong border, built-card border, disabled text |
| map canvas | `#0D1013` | map background (grid lines `#14181C` @160px; graph dot-grid `#1D2126` dots @16px) |

### Color — Ink (text)
`ink-100 #ECEEF0` primary · `ink-300 #AAB3BB` secondary · `ink-500 #6C7680` muted · `#49525B` faint/mono-hints · `#3B434B`/`#333A41` canvas ghost labels.

### Color — Signal (accent). Rule: **"orange is a verb"** — interactive or attention-demanding only, never decoration, never status.
`signal-500 #F78B23` primary · `signal-400 #FFA347` hover · pressed `#D97614` · selected bg `rgba(247,139,35,.14)` with border `rgba(247,139,35,.35)`. Text on orange: `#141414`.

### Color — Blueprint (exclusively "planned"; nothing else may be blue)
`bp-400 #56A8FF` stroke/text · `bp-600 #2A5F96` dim stroke/dashed borders · ghost fill `rgba(86,168,255,.08)` · dim text `#5A86B3` · hatch: `repeating-linear-gradient(45deg, rgba(86,168,255,.18) 0 2px, transparent 2px 5px)`.

### Color — Flow (route/belt saturation + semantic status; always paired with a redundant channel)
| Level | Hex | Dark border/companion | Threshold |
|---|---|---|---|
| OK | `#46C07A` | `#1E4531` | < 70% |
| WARN | `#E5A83B` | `#8A6423` | 70–95% |
| CRIT | `#FF5D55` | `#7A2622` | ≥ 95% / deficit |

Underclock indicator (clock < 100%): teal `#5BC8C0`, border `#2A5F57`.

### Typography
Fonts (Google Fonts): **Rajdhani** (500/600/700) — display, titles, labels, buttons; **Barlow** (400/500/600) — body/any text ≥ 2 lines; **JetBrains Mono** (400/500/700) — **every number in the app**, always `font-variant-numeric: tabular-nums`.

| Style | Spec |
|---|---|
| display-34 | Rajdhani 700, 34/1.05, uppercase |
| title-21 | Rajdhani 600, 21/1.1 |
| panel-header | Rajdhani 600 13px, uppercase, letter-spacing .12em |
| label-11 | Rajdhani 600 11px, uppercase, letter-spacing .14em |
| body-13/12 | Barlow 400, 13px/1.5 (min 12px) |
| data-22 | JB Mono 700 22px (hero rates) |
| data-12 / data-10.5 | JB Mono 500 (tables, chips); 10px floor only for canvas edge labels |

Rules: a rate's unit is attached and smaller (`480`<small>`/min`</small>); **planned/projected numbers render italic**; Rajdhani never below 11px.

### Spacing / radius / chrome
4px base scale (4·8·12·16·24·32). Row heights: 24px dense lists, 28px default, 32–36px inputs; titlebar 36px; status bar 24px; context bars 36–40px. Panel padding 12–18px. Radius: 3px default, 0 on canvas elements. Signature flourish (reserved for proposal cards, modals, primary CTAs): top-right **corner cut** via `clip-path: polygon(0 0, calc(100% - 8px) 0, 100% 8px, 100% 100%, 0 100%)` (8–12px).

### Motion (all `cubic-bezier(.2,0,0,1)`, nothing bouncy)
120ms hover/panels · 200ms drawers · 300ms solve "settle flash" (changed numbers flash amber → ink) · 400ms map↔factory zoom morph (the one cinematic moment) · built-belt flow animation: 12s slow directional drift. **Reduced-motion:** crossfades replace zoom/drawers, belt animation off, settle flash = instant swap (patterns + labels already carry the data).

### Iconography
Game icons are the nouns: extracted at runtime, never recolored/redrawn, sizes 20/28/40, always on a steel-800 chip. UI verbs (pan, filter, close, search) = one geometric 1.5px-stroke line set, ink-300. An item is never text-only where an icon can lead.

---

## The Status System (used identically on every surface — see mock `1d`)
Grammar = stroke + fill + glyph; color reinforces but never carries alone.

| State | Glyph | Stroke | Fill | Numbers |
|---|---|---|---|---|
| Planned | ◇ | dashed `#56A8FF` | blueprint hatch / `rgba(86,168,255,.05-.08)` | italic ("projected") |
| Under construction | ◈ | dashed `#E5A83B` | bottom-half fill `#8A6423` + 3px progress bar | normal, `n/target` |
| Built | ◆ | solid (`#49525B` cards / `#F78B23` pins) | solid steel | normal |

Applies to: map pins (18–22px rotated-square diamonds + mono label chip), graph cards, table rows (status chip leads the row), and routes (planned routes are always blueprint-dashed regardless of load).

## Flow/Route Encoding (mock `1e`)
Three channels agree — color + thickness + pattern, plus an always-present mono label chip `n/capacity · %`:
- OK: 2px solid `#46C07A`
- WARN: 4px dashed `10 5` `#E5A83B`
- CRIT: 6px hazard-dashed `6 4` `#FF5D55`; label chip inverts (dark text on red) with ⚠
- Planned: 2px dashed `8 6` `#56A8FF`, italic chip
- Route types: belt = single line; rail = double line (6px line with 2.5px canvas-color overlay); drone = dotted. Trend arrows (↗) on labels where load is rising.

---

## Navigation Model (decided: mock `1g` + `1i`)
**The map IS the app. No pages.** Everything is a layer or a zoom level:
- Audit = bottom drawer (pull up, half-height, pinnable to full; TAB toggles it as a HUD)
- Advisor = right edge tab (26px, badge count) expanding to a 340px panel
- Factory view = zoom continuation of the map (L0 world → L1 region mini-cards → L2 factory graph, 400ms morph; ESC zooms back out; position never lost)
- Wizard & proposal review = modal / mode over the live map
- Custom titlebar (36px, Tauri custom frame): logo square, app name, breadcrumb (`WORLD MAP / COASTAL STEEL`), save-sync chip, solver status, min/max/close.
- Status bar (24px): power `4.2/6.0 GW` + mini bar, ◈ U/C count, ⚠ deficits (clickable), counts; right side totals.
- Design floor: 1920×1080 fixed; density = node-editor grade (Blender/TradingView).

---

## Screens

### 1. Map home (mock `2a`, 1920×1080)
- Map canvas between titlebar (36) and status bar (24). Real extracted map imagery; 160px survey grid; biome labels in 10px mono `#3B434B`.
- Top-left: search field 280×30 (`⌘K — find item, factory, node…`).
- Top-center: overlay chips 30px high — FLOWS / POWER / DIFF / NODES, keys 1–4; active = orange fill, dark text. FLOWS/POWER/NODES are additive; **DIFF is a mode** (map desaturates, planned renders blue, drift outlines amber with delta chips).
- Top-right: `+ FACTORY (N)` ghost button · `PLAN SUPPLY CHAIN (P)` primary orange · zoom control (− / % / +).
- Resource nodes: 14px circles — purity via ring style (pure solid / normal dashed / impure dotted); claimed = orange center dot; free = hollow gray; conflict = red ring + red label + `⚠×2`.
- Factory pins: 22px diamonds per status grammar, mono label chips below (`◆ NORTHERN FORGE`), 4px canvas-colored halo (`box-shadow: 0 0 0 4px rgba(13,16,19,.85)`).
- Bottom-left: collapsible legend (230px) — status glyphs, load samples, route types, node states.
- Bottom-center: `▲ AUDIT (TAB)` handle. Right edge: advisor tab with orange badge.
- **Interactions:** hover pin → its routes stay full color, rest dims to 40%; click = select → summary drawer; double-click/⏎ = dive; scroll = zoom to cursor; planned pins drag to relocate (routes re-measure live), built pins locked (offer "plan a move"); right-drag from pin/node = draw route (ghost-blue until confirmed, snaps to valid targets). Keys: ⌘K, 1–4, TAB, A advisor, N, P, F frame, ESC deselect/zoom-out.

### 2. Factory summary drawer (mock `2b`, 380px, slides over map right edge, 200ms)
Header (icon 36, name title-17, location/machines/nodes mono sub, status chip, ×) → OUTPUTS (icon + name + data-14 rate) → ROUTES IN/OUT (each: direction arrow + source + tier, 44px saturation mini-bar, mono load, ⚠ if capped) → POWER DRAW + PLAN-vs-BUILT (`MATCHES PLAN ✓` green or drift delta) → footer: `OPEN FACTORY ⏎` (primary, 34px) · HISTORY · ⋯ overflow.

### 3. Advisor panel (mocks `2c` feed / `6a` chat, 340–420px, right side)
Header: ADVISOR + `AMBIENT · ON` chip + last-eval time + pause + collapse. Two tabs: FEED (badge) / CHAT.
- **Feed cards** (steel-800, corner-cut): severity chip (`⚠ CONFLICT` red / `▲ TREND` amber / `● TIP` gray — dark text on colored chip) + Rajdhani-13 title + Barlow body with inline mono values + actions (`CREATE PROPOSAL`/`RESOLVE…` orange-outline, `DISMISS` ghost) + provenance footer line: `SAW: <inputs> @<time> · RULE: <heuristic>` in 9px mono `#49525B`. Panel footer: "The advisor never edits your plan — every suggestion becomes a proposal you review."
- **Chat**: context bar (scope selector EMPIRE/FACTORY/SELECTION, snapshot time, payload size, "▸ view" shows exact JSON sent). User msgs = right-aligned steel-700 bubbles; AI answers = full-width, may include a mono causal-chain block (numbered lines, color-coded by severity) and clickable entity chips (`◆ COASTAL STEEL`) that highlight on the map. Streaming = orange block cursor + `STREAMING…`. Every answer: SAW provenance line. Composer: `@` references entities; footnote "Answers can propose changes — they arrive as proposals, never as edits."
- **Gating (behavior):** local heuristics arm it (new deficit, conflict, saturation >75%, power swing), 30s debounce, visible hourly call budget, per-rule mute on dismiss. Offline/no key: `AI OFFLINE` chip; local heuristic warnings keep flowing into the same feed.

### 4. Audit drawer (mock `2d`, full width, half-height, pinnable)
Tabs with count badges: SATURATION / DEFICITS / POWER / PLAN DRIFT + `LIVE — re-audits on every change` + pin/collapse. Dense 32px rows, 7-col grid (route Rajdhani-12 / type·tier mono / load % / throughput bar 70px / projection Barlow / trend mono / actions). CRIT rows get `rgba(255,93,85,.05)` row tint. Row actions: `FIX WITH SOLVER` (pre-fills wizard), `TRACE`, `UPGRADE TIER`, `SPLIT ROUTE`. Click row = highlight route on map behind; ⏎ = jump to it. Footer: sort note in 10px mono.

### 5. Proposal review (mock `3a`, a MODE over the live map — the shared trust surface)
- Banner (40px, 2px orange bottom border): `PROPOSAL #7` stamp (corner-cut), goal title, provenance (`GLOBAL SOLVER · 8.4s · SNAPSHOT 14:02 · NOTHING APPLIES UNTIL YOU ACCEPT`), `EXIT REVIEW · ESC`.
- Map behind: existing empire dims to 42% opacity; proposal renders in status grammar — new = blueprint ghost pin with inverted blue `+ NAME — NEW` chip; modified = 40px dashed amber ring around pin + `Δ` chip; claims = dashed blue ring around node; new routes = blueprint dashed with italic projection chips (`◇ MK.4 · proj 62% · 1.2km`).
- Left panel (470px, `rgba(21,24,27,.97)`): grouped change list — CREATE (blue header) / MODIFY (amber) / CLAIM / ROUTE. Each row: 14px checkbox (checked = orange, dark ✓), type chip (`+`/`Δ`/`◉`/`⟶`), entity name Rajdhani-13 + status chip, one-line detail (Barlow 11), right-aligned mono impact (`+96 MW`, `FREE ✓`, `62%`).
- **Partial accept:** excluded rows drop to 55% opacity, name struck through, consequence recomputed **live** and shown both in-row (`⚠ 96%`) and as an amber warning strip ("1 warning from exclusions — … Goal still met.").
- Impact footer: 4-cell grid (GOAL CHECK `8.0/8.0 ✓` / Δ POWER / MACHINES / BUILD COST ▸) + `ACCEPT 7 AS PLANNED ⏎` (corner-cut primary) · `RE-SOLVE…` · `REJECT` (red text ghost) + microcopy "Accepting creates ◇ planned entities only — one undo step. The built layer is never touched."
- Detail popover per MODIFY item (mock `3b`): BEFORE → AFTER table, changed rows tinted `rgba(229,168,59,.05)` with amber italic AFTER values, unchanged rows dim; WHY line; INCLUDED ✓ / EDIT VALUES / EXCLUDE.
- **Lifecycle invariants (mock `3c`):** DRAFT → REVIEWING → ACCEPTED (→ ◇ plan) | REJECTED (archived, recallable). Never auto-apply; accept = one undo step; plan changed while open → `STALE` badge + one-click re-solve; every item editable before accept; provenance (source, snapshot, input hash) on every proposal. Keys: ↑↓ walk (map pans along), SPACE toggle, E edit, D diff, ⏎ accept, ESC exit (draft kept).

### 6. Factory graph view (mock `4a`, 1920×1080; React Flow)
- Context bar (36px below titlebar): `⟵ WORLD · ESC` chip, factory name + status chip, stats (`186 MW → 214 ◇` — planned deltas italic blue), `◇ Δ#7 — 2 ITEMS PENDING BUILD` chip, local overlay toggles FLOW/POWER/DIFF.
- Canvas: steel-950 with 16px dot grid. **Boundary ports** = slim cards (200px) at left (INPUT) / right (OUTPUT) edges carrying route context in from the map (tier chip, source/destination, capped state in red).
- **Machine-group cards** (240–270px, anatomy in `4b`): header (game icon 22 + `FOUNDRY ×12` Rajdhani-14 + clock chip — teal ↓ underclocked / orange overclocked), recipe row (icon + name, ◇ badge if swap planned), footer `IN n / OUT n / power` mono. Selected = 2px orange border + corner cut. Planned additions = ghost sub-cards (dashed blue, hatch, italic, `Δ#7` provenance tag) connected by dashed blue edge — **built and planned coexist on one canvas**.
- Edges: same flow encoding as map; mono labels on canvas-color chips.
- **Inspector** (right, 360px, on selection): OUTPUT TARGET slider (orange fill, diamond handle, tick at input ceiling — hard-stops there; binding constraint highlights red with a fix-suggestion strip) + `RE-SOLVES LIVE ON DRAG · LAST 2.8ms ✓`; CLOCK segmented 50/75/100/150/250 + fine % input + shard note; I/O table with per-input saturation bars; FEED BELTS tier dropdowns (`MK.3 ▾ UPGRADE?` amber when capped); footer: "Edits apply instantly to the plan. On a ◆ built bank they become ◇ deltas — visible in DIFF until built in-game."
- **Recipe strip** (bottom-center, contextual on selection, 640px): build-menu style tiles 104px — current (orange border), planned alt (dashed blue + `◇ Δ#7`), available (steel), locked (50% opacity, `NOT UNLOCKED`). Hover = ghost preview on graph. Key R.
- Minimap bottom-left (170px): region + pin dot + `ESC ⟶ WORLD`.
- **Solver contract (mock `4c`):** every edit re-solves live (<5ms budget, time always visible); **numbers change, geometry doesn't** — a solve never adds/moves/removes cards; changed values settle-flash 300ms; infeasible input hard-stops sliders instead of erroring. Direct manipulation: drag card (16px snap), drag port = new belt (tier picker on drop), alt-drag = split bank, ⌫ planned-only (built asks "plan a demolition?"), double-click canvas = add group. Keys: R/T/C/B, / find, F, ⌘Z (includes solves), ESC.

### 7. Supply-chain wizard (mocks `5a` goal / `5b` solve; 880px corner-cut modal over dimmed map)
- Header: `WIZARD` stamp + title + step chips `1 GOAL / 2 SOLVE / 3 REVIEW` (done = green ✓ outline, active = orange fill).
- Step 1: goal sentence — `PRODUCE [icon + item ▾] AT [8.0 /min] EMPIRE-WIDE` + `+ ADD GOAL`; quick-fill chips from live state (deficits in red). Constraints grid (2-col, toggles are 28×16 switches): surplus-first, transport multi-select chips, max new sites, node budget + purity floor, unlocked alts on / locked-alts-suggest off, power-margin cap, EXPAND↔GREENFIELD preference slider. Footer: `SOLVE ⏎` + `TYPICALLY 5–15s · RUNS LOCALLY` + "Result is a proposal — nothing is applied until you review it."
- Step 2: phase list (DEMAND GRAPH ✓ 0.4s → RECIPE SELECTION ✓ → SITING active → ROUTING pending) + overall bar + **live solver log** (real output: alts picked, surplus consumed, site candidates scored, penalties) on steel-950; candidate sites ghost-flicker on the map behind. `SOLVE IN BACKGROUND` (lands as status-bar chip) / `CANCEL` (instant, stateless).
- Step 3 **is** the proposal review surface (screen 5) with wizard breadcrumb. `RE-SOLVE…` returns to step 1 with constraints intact; drafts numbered and archived.
- **Infeasible ≠ dead end (mock `5c`):** return best-achievable rate + binding constraint + one-tap relaxations with costs ("allow 1 more node claim → 8.0 ✓"). Entry points, all pre-filling the goal: P key, map button, any `FIX WITH SOLVER` row, advisor CTA, right-click item icon → "plan production."

### 8. Image → style guide (mock `6b`, 1080px surface)
Header: `STYLE GUIDE` stamp + generated name (`"TERRACED BRUTALIST" — FROM SCREENSHOT`) + `GENERATED <time> · 1 IMAGE · AESTHETIC INFERENCE, NOT A COPY`. Left column (360px): uploaded screenshot, `+ ADD ANGLES` dashed slot, detected-attribute chips, disclosure "Sent to the model: this image + your unlocked building parts list. Nothing else." Right column: MATERIAL PALETTE (icon + material + proportion bar + %), MASSING & RHYTHM prose, SIGNATURE TECHNIQUES numbered list, CONSTRUCTION SEQUENCE (5 numbered step-cards with per-step materials), footer `SAVE TO LIBRARY` / `SET AS FACTORY THEME` / `TOTAL MATERIALS LIST ▸`.

### 9. Onboarding (mocks `7a`/`7b`/`7c`; 880px modals, 4 step chips)
- Steps 1–2: auto-detected install path (mono, in a field) + version chip + CHANGE…; extraction progress (`1,204/2,410` data-13, 5px bar, icon grid filling live, per-category mono counters). Fallback: `SKIP — USE COMMUNITY PACK`. Copy: install is source of truth, re-extracts on game patches.
- Step 3 (OPTIONAL chip): .sav drop zone + detected saves; amber honesty strip ("format is community-reverse-engineered… skip — everything works with manual entry"); parse preview table (factories/machines/belts/trains/power + "unrecognized: 17 → listed, ignored"); "Everything imports as ◆ BUILT. Your plan is never touched — future re-imports diff against built." CTAs: `IMPORT AS BUILT` / `SKIP — START MANUAL`.
- Step 4: land on the live empty map (nodes visible, unclaimed) with one centered card, three doors: N place first factory / P plan a supply chain / S import save. Footer: "NO TOUR. THE MAP TEACHES BY DOING — ⌘K WHENEVER LOST."

---

## Interaction Principles (mock `7d` — binding, product-wide)
1. **The map is truth.** Every surface is a layer on it or a zoom level of it; position is never lost.
2. **Solver output is always editable.** A solve is a starting point; grab any number and it re-solves around you.
3. **Nothing mutates without review.** Solver, advisor, chat, save re-import — all changes arrive as proposals with provenance; accept is one undo step.
4. **◇ ◈ ◆ everywhere.** Status reads identically on map, graph, tables — stroke + fill + glyph, never color alone.
5. **Numbers change, geometry doesn't.** Solves never move cards or pins.
6. **No dead ends.** Infeasible → best-achievable + relaxations; parse failure → manual; AI offline → local heuristics.
7. **Orange is a verb.** Signal marks actions/attention only; status = Flow colors, planned-ness = Blueprint blue.
8. **Silence is a feature.** The advisor's loudest voice is a badge count.

## State Management (implementation notes)
- Every entity (factory, machine group, route, node claim, power segment) carries `status: planned | under_construction | built` plus optional `plannedDelta` refs (a built entity can hold ◇ deltas — see `4a`).
- Proposals: `{ id, source: solver|advisor|chat|save_reimport, goal, snapshotTime, inputHash, items[{ kind: create|modify|claim|route, included, payload, consequences }] , state: draft|reviewing|accepted|rejected, stale }`. Exclusion toggles trigger live consequence recompute. Accept = single undoable transaction creating planned entities.
- Local solver runs on every inspector/slider/recipe/tier edit (<5ms budget; display solve time). Global solver runs async with phase/log streaming; cancellable; backgroundable.
- Advisor: event gate (deficit/conflict/saturation>75%/power swing) → 30s debounce → budget check → call; provenance stored per card. Chat context scope: empire/factory/selection snapshots.
- Save import writes only the built layer; re-import produces a diff proposal.
- Persistence: plan file + archived proposals + dismissed-rule mutes + window/zoom position.

## Assets
- **No assets ship in the binary.** Item/building icons, recipe data, and map imagery are extracted from the user's local game install on first run (step `7a`); community icon pack is the fallback; re-extract on game patch.
- Every icon/map placeholder in the mocks is a 45° diagonal-stripe block — replace with the extracted asset at the annotated size (20/28/40 for icons).
- Fonts: Rajdhani, Barlow, JetBrains Mono (Google Fonts; bundle locally in the Tauri app).

## Files
- `FICSIT Planner — Foundations.dc.html` — the complete design reference. Open in a browser. Sections (newest first): Turn 7 onboarding + principles (`7a–7d`), Turn 6 AI layer (`6a–6c`), Turn 5 wizard (`5a–5c`), Turn 4 factory view (`4a–4c`), Turn 3 proposal review (`3a–3c`), Turn 2 map home (`2a–2e`), Turn 1 tokens/status system/nav options (`1a–1i`; nav decision = `1g` + `1i`'s TAB HUD). Every spec card's text is normative, not decorative.
