# Design Brief: FICSIT Planner — Satisfactory Factory & Logistics Planner

You are the design lead for a desktop application. Your job in this session is to produce a complete, opinionated UI/UX design system and screen-by-screen design specification. Dig deep: question weak assumptions, propose better interaction patterns where you see them, and make deliberate visual choices — not templated defaults. Ask me clarifying questions before producing major deliverables when a decision would fork the design.

## What the product is

A desktop factory and logistics planner for the game Satisfactory (Tauri + React + Rust — same stack as my project Wire). It competes with satisfactory-calculator.com and Satisfactory Tools, and wins on UX: their output is a static report; ours is a living blueprint you keep editing.

Core positioning: **the map is the source of truth.** Factories are not abstract ratio calculations — they are placed entities on the world map that claim specific resource nodes and connect via routes (belts, trains, trucks, drones) with real distances. Power is another network layered on the same map. This lets the app answer questions no ratio calculator can: belt saturation between plants, train round-trip throughput, double-booked nodes, grid brownout risk.

## Product decisions already made (design within these)

1. **Map-aware, whole-empire scope.** Interactive world map with resource nodes, factory pins, routes, power overlay.
2. **Dual state model: planned vs. built.** Every entity carries a status (planned / under construction / built). The diff between plan and reality is a first-class audit surface.
3. **Two solvers.**
   - *Local solve* (default verb): solve one factory's internals at its location. Instant — re-runs live on every slider drag or recipe swap.
   - *Global solve*: an explicit "Plan a supply chain" wizard ("I want 20 HMF/min empire-wide"). May take seconds; returns a reviewable **proposal** (new factories, node claims, train lines) the user accepts or modifies. Never silently mutates the plan.
4. **AI layer (Claude API), strictly additive** — app is fully functional offline as a solver:
   - *Ad-hoc chat*: ask anything with full empire state as context.
   - *Ambient advisor* (opt-in): re-evaluates on meaningful state changes (debounced, gated by cheap local heuristics), surfaces prioritized recommendations — "what to build next," deficit warnings, alt-recipe opportunities. This is the decision-paralysis killer feature.
   - *Image-to-style-guide*: user uploads a screenshot of a community build; the app returns an aesthetic system (material palette, massing, signature techniques) plus a construction sequence and materials list — the vibe, not tile-for-tile placement.
   - AI and global solver share one **pending changes / proposal review** system.
5. **World data: manual entry + save import.** Save (.sav) parsing enriches the *built* layer; it never clobbers the plan. Manual-first posture since the format is community-reverse-engineered and brittle.
6. **Assets:** on first run, point the app at the local game install and extract item/building icons and recipe data directly (user owns the game; nothing ships in our binary; app self-updates on game patches). Community icon pack as fallback. Players think in icons, not names — icon-first UI everywhere.

## Established visual direction

FICSIT industrial: the game's own visual language so there is zero learning curve. Dark steel surfaces, signal orange accent, condensed industrial display type (Rajdhani or similar), monospace for all numbers, hazard-stripe motifs used sparingly. A first mockup of the factory view exists and validated: machine-group cards on a dot-grid canvas, belts as animated edges color-coded by saturation (green/amber/red vs. belt tier capacity), live-solving inspector panel with a target-rate slider, recipe picker strip styled like the in-game build menu. Keep this direction; refine and systematize it.

## Surfaces to design (screen-by-screen spec expected)

1. **Map (home).** Pan/zoom world map, node icons, factory pins, route lines, toggleable overlays (power grid, item flows, planned-vs-built diff). Click pin → summary drawer → dive into factory.
2. **Factory view.** Node-graph editor (React Flow), machine cards, live local solver, alt-recipe swapping, clock/underclock controls, belt tier selection.
3. **Dashboard (audit).** Empire-wide deficits, belt saturation hotspots, power margin, plan/built drift, advisor feed.
4. **"Plan a supply chain" wizard.** Goal input → solver progress → proposal review (diff-style: what gets created/claimed/routed) → accept/modify/reject.
5. **Proposal review system.** Shared by wizard and AI advisor. Design this once, well.
6. **AI advisor.** Both the ambient feed presence (must inform without nagging) and ad-hoc chat. Design the trust affordances: why is it recommending this, what state did it see.
7. **Image-to-style-guide flow.** Upload → analysis → generated style guide + build sequence presentation.
8. **Onboarding.** Game-install detection, asset extraction progress, optional save import, first factory creation.

## Deliverables

1. Design tokens: full palette, type scale, spacing, iconography rules, motion principles (respect reduced-motion).
2. Component inventory: cards, chips, drawers, proposal diffs, graph edges/labels, map pins, overlay legends.
3. Screen-by-screen specs with layout wireframes and interaction notes (what happens on hover, click, drag, keyboard).
4. Interaction principles doc: the "living blueprint" philosophy — solver output is always directly editable, proposals never auto-apply, planned vs. built always distinguishable at a glance.
5. Where valuable, working React mockups of the highest-risk screens (map, proposal review).

## Known tensions to resolve (bring a point of view)

- Map vs. graph: how does the user move between spatial (map) and logical (factory graph) views without losing context?
- Density: this is a power-user tool; err toward information density, but define where breathing room is non-negotiable.
- The ambient advisor's presence: persistent sidebar, notification tray, or dashboard-only? Naggy is fatal; invisible is useless.
- How planned/under-construction/built states read visually across map, graph, and dashboard simultaneously.

Start by restating the product in your own words, list the design risks you see, ask me anything that would fork the direction, then propose the token system before going screen by screen.
