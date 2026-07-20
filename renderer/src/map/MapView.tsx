// Map home (mock 2a). The map IS the app: everything else is a layer or zoom
// level. Leaflet CRS.Simple; canvas layer for grid/labels/nodes; DOM pins for
// factories; placeholder treatment for world imagery (runtime asset — see
// DECISIONS.md on tile licensing).

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { MapCanvasLayer } from "./CanvasLayer";
import { attachSmoothWheelZoom } from "./smoothZoom";
import type { CanvasLayerData, NodeRenderState } from "./CanvasLayer";
import { fromLatLng, toLatLng } from "./maputil";
import { motionKind } from "../graph/graphMotion";
import { CLUSTER_CAP, CLUSTER_STEP_MS, CONVERGE_MS, scatter, tetherKey, type MapMotion } from "./mapMotion";
import { useStore } from "../state/store";
import Glyph from "../lib/glyphs";
import { isEditableTarget } from "../lib/keys";
import type { WorldNode } from "../state/types";
import SummaryDrawer from "./SummaryDrawer";
import NodeDrawer from "./NodeDrawer";
import ResourceOverview from "./ResourceOverview";
import RouteDrawer from "./RouteDrawer";
import SwitchDrawer from "./SwitchDrawer";
import RoutePopover from "./RoutePopover";
import Legend from "./Legend";
import SearchBox from "./SearchBox";
import { fmtPower, itemLabel, routeBottleneck } from "../lib/format";
import "./map.css";

/** Cargo route kinds drawn with the saturation line grammar (A3.1). Pipe is
 *  excluded: not creatable in the UI and it has no derived flow/capacity. */
const CARGO_KINDS = new Set(["belt", "rail", "truck", "drone"]);

/** Circuit margin level (SDD §12): headroom ≥20% OK, 5–20% WARN, <5% CRIT. */
function circuitLevel(genMw: number, demandMw: number): "ok" | "warn" | "crit" {
  if (genMw <= 0) return demandMw > 0 ? "crit" : "ok";
  const headroom = (genMw - demandMw) / genMw;
  if (headroom < 0.05) return "crit";
  if (headroom < 0.2) return "warn";
  return "ok";
}

function pinHtml(name: string, status: string, selected: boolean, tag?: "retiring" | "incoming"): string {
  const glyph = status === "planned" ? "◇" : status === "under_construction" ? "◈" : "◆";
  // 22px rotated-square diamond as SVG — dashed strokes stay crisp
  const stroke =
    status === "planned" ? "var(--bp-400)" : status === "under_construction" ? "var(--flow-warn)" : "var(--signal-500)";
  const dash = status === "built" ? "" : 'stroke-dasharray="4 3"';
  const fill =
    status === "built" ? "var(--steel-800)" : status === "planned" ? "rgba(86,168,255,.10)" : "rgba(138,100,35,.4)";
  // W2a: a retiring ◆ / incoming ◇ carries a cutover tag chip.
  const tagChip =
    tag === "retiring"
      ? `<div class="pin-tag mono retiring">RETIRING</div>`
      : tag === "incoming"
        ? `<div class="pin-tag mono incoming">INCOMING</div>`
        : "";
  return `
    <div class="pin-wrap ${tag ? `cutover-${tag}` : ""}">
      <svg class="pin-svg ${selected ? "selected" : ""}" width="30" height="30" viewBox="0 0 30 30">
        <rect x="8" y="8" width="14" height="14" transform="rotate(45 15 15)"
              fill="${fill}" stroke="${stroke}" stroke-width="2" ${dash} />
      </svg>
      <div class="pin-chip mono ${status}">${glyph} ${name.toUpperCase()}</div>
      ${tagChip}
    </div>`;
}

/** Pin-chip declutter: chips whose rects would overlap an already-kept chip
 *  are culled (the diamond stays; hovering the pin reveals the name).
 *  Selected factory wins, then stable name order — the same zoom always
 *  shows the same chips. */
function declutterPinChips(map: L.Map, markers: Map<string, L.Marker>) {
  const st = useStore.getState();
  const entries = Object.values(st.plan.factories)
    .map((f) => {
      const marker = markers.get(f.id);
      const el = marker?.getElement()?.querySelector(".pin-chip") as HTMLElement | null;
      if (!marker || !el) return null;
      const pt = map.latLngToContainerPoint(toLatLng(f.position));
      // Real rendered width when visible (covers status glyphs, warn badges,
      // RETIRING/INCOMING tags the old estimate missed — the source of visibly
      // overlapping labels in dense clusters). Culled chips are display:none
      // and read offsetWidth 0, so cache the last visible measure — otherwise
      // a culled chip shrinks to the estimate, wins the next pass, and the
      // pair oscillates. A rename while hidden leaves the cache stale for one
      // cycle; it self-corrects on the next visible measure.
      if (el.offsetWidth) el.dataset.w = String(el.offsetWidth);
      const w = el.offsetWidth || Number(el.dataset.w) || f.name.length * 6.4 + 34;
      return {
        el,
        selected: st.selection?.kind === "factory" && st.selection.id === f.id,
        name: f.name,
        rect: { x: pt.x - w / 2, y: pt.y + 15, w, h: 20 },
      };
    })
    .filter((e): e is NonNullable<typeof e> => e !== null)
    .sort((a, b) => (a.selected !== b.selected ? (a.selected ? -1 : 1) : a.name.localeCompare(b.name)));
  const kept: { x: number; y: number; w: number; h: number }[] = [];
  for (const e of entries) {
    const hit = kept.some(
      (k) => e.rect.x < k.x + k.w && k.x < e.rect.x + e.rect.w && e.rect.y < k.y + k.h && k.y < e.rect.y + e.rect.h,
    );
    if (!hit) kept.push(e.rect);
    e.el.classList.toggle("chip-culled", hit);
  }
}

export default function MapView() {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const layerRef = useRef<MapCanvasLayer | null>(null);
  const markersRef = useRef<Map<string, L.Marker>>(new Map());

  const plan = useStore((s) => s.plan);
  const world = useStore((s) => s.world);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const overlays = useStore((s) => s.overlays);
  const mapFilter = useStore((s) => s.mapFilter);
  const selection = useStore((s) => s.selection);
  const placing = useStore((s) => s.placingFactory);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const setOverlay = useStore((s) => s.setOverlay);
  const setPlacing = useStore((s) => s.setPlacingFactory);
  const dispatch = useStore((s) => s.dispatch);
  const reviewing = useStore((s) => s.reviewing);
  const setWizard = useStore((s) => s.setWizard);
  const reviewingProposal = useStore((s) => (s.reviewing ? s.plan.proposals[s.reviewing] ?? null : null));

  const [hoveredNode, setHoveredNode] = useState<WorldNode | null>(null);
  const [zoomPct, setZoomPct] = useState(100);
  const [routeDraft, setRouteDraft] = useState<{ from: string; cursor: { x: number; y: number } } | null>(null);
  const [routePopover, setRoutePopover] = useState<{ from: string; to: string } | null>(null);
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);
  // #117 toolbar rework: the node/factory search renders into the titlebar's
  // CENTERED slot — context-aware: the factory graph portals its own
  // machine/item search into the same slot. (The DATA menu is Titlebar-owned
  // now — see shell/DataMenu.tsx.) useLayoutEffect so the slot resolves before
  // paint — a passive effect would flash the in-map fallback for one frame.
  const [searchSlot, setSearchSlot] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => {
    setSearchSlot(document.getElementById("titlebar-search-slot"));
  }, []);
  // Drag-drop stays map-owned (the map is the drop surface); the files land in
  // the store, where the Titlebar's DataMenu renders the ImportModal.
  const acceptFiles = useCallback((files: File[]) => {
    const sav = files.find((f) => f.name.toLowerCase().endsWith(".sav"));
    const docs = files.find((f) => f.name.toLowerCase().endsWith(".json"));
    void (async () => {
      const s = useStore.getState();
      if (docs && __WASM_BACKEND__) {
        const bytes = new Uint8Array(await docs.arrayBuffer());
        // awaited so a docs+save drop processes in the required order
        await s.uploadDocs(bytes);
      }
      if (!sav) return;
      // The Docs→save order is ENFORCED on web (a fixture-catalog import
      // quarantines most recipes): refuse the save until a catalog is loaded.
      const bv = useStore.getState().gamedata.buildVersion;
      if (__WASM_BACKEND__ && (!bv || bv === "fixture")) {
        s.pushToast("Load your Docs.json first (DATA ▾ step ①) — then drop your save", "error");
        return;
      }
      useStore.getState().setImportFile(sav);
    })();
  }, []);
  const routeDraftRef = useRef<typeof routeDraft>(null);
  routeDraftRef.current = routeDraft;
  // one-shot: swallow the contextmenu that follows a route-drag release
  const suppressCtxRef = useRef(false);

  // Resolved node set (W2b-C): catalog nodes at their plan-corrected position,
  // plus save-only nodes (`save:<id>`, absent from every catalog) synthesized
  // from their override alone. The bundled asset stays an ambient default —
  // this overlay never mutates `world.nodes`.
  const resolvedNodes = useMemo(() => {
    const catalog = new Set(world.nodes.map((n) => n.id));
    const out = world.nodes.map((n) => {
      const ov = plan.nodeOverrides[n.id];
      return ov?.pos ? { ...n, x: ov.pos.x, y: ov.pos.y, z: ov.pos.z ?? n.z } : n;
    });
    for (const [id, ov] of Object.entries(plan.nodeOverrides)) {
      if (!catalog.has(id) && ov.pos) {
        out.push({
          id,
          item: "",
          purity: "normal",
          x: ov.pos.x,
          y: ov.pos.y,
          z: ov.pos.z ?? 0,
          zone: "surface",
          region: "",
        });
      }
    }
    return out;
  }, [world.nodes, plan.nodeOverrides]);
  const resolvedWorld = useMemo(
    () => ({ ...world, nodes: resolvedNodes }),
    [world, resolvedNodes],
  );

  // Live search filter over the map: typing narrows the visible nodes by
  // resource type / purity. Matching is by the extracted item's display name
  // (so "iron", "coal", "quartz" work) plus purity ("pure"/"normal"/"impure").
  // If the query matches NO node (e.g. a factory name), the filter goes inert
  // so searching a factory never blanks the resource field.
  const nodeFilter = useMemo(() => {
    const q = mapFilter.trim().toLowerCase();
    if (!q) return null;
    const visible = new Set<string>();
    for (const n of resolvedNodes) {
      const label = itemLabel(gamedata.items, n.item).toLowerCase();
      if (label.includes(q) || (n.purity ?? "").toLowerCase().includes(q)) visible.add(n.id);
    }
    return { active: visible.size > 0, visible };
  }, [mapFilter, resolvedNodes, gamedata.items]);

  const claimLinks = useMemo(() => {
    const nodeById: Record<string, { x: number; y: number }> = {};
    for (const n of resolvedNodes) nodeById[n.id] = { x: n.x, y: n.y };
    const links: CanvasLayerData["claimLinks"] = [];
    for (const c of Object.values(plan.nodeClaims)) {
      const node = nodeById[c.node];
      const f = plan.factories[c.factory];
      if (!node || !f) continue;
      // A search-hidden node draws no marker — its tether would point at
      // empty ground, so it hides with it.
      if (nodeFilter?.active && !nodeFilter.visible.has(c.node)) continue;
      links.push({
        node,
        factory: f.position,
        factoryName: f.name,
        planned: c.status === "planned",
        conflict: derived.nodes[c.node]?.conflict ?? false,
        highlight:
          (selection?.kind === "factory" && selection.id === c.factory) ||
          (selection?.kind === "node" && selection.id === c.node) ||
          hoveredNode?.id === c.node,
      });
    }
    return links;
  }, [plan.nodeClaims, plan.factories, resolvedNodes, derived.nodes, selection, hoveredNode, nodeFilter]);

  // Refactor tethers (W2a): old ◆ → new ◇ links from every `replaces`. Highlight
  // when either endpoint is the selected factory ("orange is a verb").
  const replacesLinks = useMemo(() => {
    const links: CanvasLayerData["replacesLinks"] = [];
    for (const f of Object.values(plan.factories)) {
      if (!f.replaces) continue;
      const old = plan.factories[f.replaces];
      if (!old) continue;
      links.push({
        old: old.position,
        new: f.position,
        highlight:
          selection?.kind === "factory" && (selection.id === f.id || selection.id === old.id),
      });
    }
    return links;
  }, [plan.factories, selection]);

  const nodeStates = useMemo(() => {
    const out: Record<string, NodeRenderState> = {};
    const claimsByNode: Record<string, number> = {};
    for (const c of Object.values(plan.nodeClaims)) {
      claimsByNode[c.node] = (claimsByNode[c.node] ?? 0) + 1;
    }
    for (const n of resolvedNodes) {
      const claims = claimsByNode[n.id] ?? 0;
      out[n.id] = {
        claims,
        claimed: claims > 0,
        conflict: derived.nodes[n.id]?.conflict ?? false,
        drift: derived.nodes[n.id]?.drift ?? false,
      };
    }
    return out;
  }, [plan.nodeClaims, resolvedNodes, derived.nodes]);

  // ---- map init (once) ----
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;
    const map = L.map(containerRef.current, {
      crs: L.CRS.Simple,
      zoomControl: false,
      attributionControl: false,
      minZoom: 1,
      maxZoom: 6,
      // zoomSnap 0 lets the eased wheel zoom land on any fractional level;
      // scrollWheelZoom off hands the wheel to attachSmoothWheelZoom (below) so
      // Leaflet's stepped handler doesn't double-zoom.
      zoomSnap: 0,
      scrollWheelZoom: false,
      doubleClickZoom: false,
    });
    const b = useStore.getState().world.bounds;
    const bounds = L.latLngBounds(toLatLng({ x: b.minX - 500, y: b.maxY + 500 }), toLatLng({ x: b.maxX + 500, y: b.minY - 500 }));
    map.setMaxBounds(bounds.pad(0.2));
    const saved = useStore.getState().viewState.map;
    if (saved) map.setView(saved.center, saved.zoom);
    else map.fitBounds(bounds);

    const layer = new MapCanvasLayer({
      world: useStore.getState().world,
      nodeStates: {},
      claimLinks: [],
      replacesLinks: [],
      hoveredNode: null,
      selectedNode: null,
      showNodes: true,
      nodeFilter: null,
      showTerrain: useStore.getState().overlays.terrain,
      routes: [],
      showRoutes: true,
      powerLines: [],
      circuitChips: [],
      switches: [],
      showPower: true,
      ghost: null,
      review: null,
      motion: null,
    });
    layer.addTo(map);
    layerRef.current = layer;
    map.on("move zoom viewreset", () => declutterPinChips(map, markersRef.current));
    mapRef.current = map;
    setZoomPct(Math.round(Math.pow(2, map.getZoom() - 2) * 100));

    // Live zoom stamp: a direct DOM write on every zoom frame (NOT React state —
    // the eased wheel zoom fires `zoom`/`moveend` per rAF frame, so a setState or
    // a persistence write per frame would re-render this heavy component / spam
    // the backend and undo the smoothness). This attribute lets the smooth-zoom
    // e2e observe the glide; the % readout + persistence land on the debounced
    // settle below.
    const stampZoom = () => {
      const rootEl = map.getContainer().closest<HTMLElement>('[data-testid="map-root"]');
      (rootEl ?? map.getContainer()).dataset.zoom = map.getZoom().toFixed(3);
    };
    map.on("zoom", stampZoom);
    stampZoom();
    // Testability stamp (M5): world-coord center on the map-root element — a
    // cheap string piggybacked on the settle we already handle, so the fly
    // e2e can assert the camera actually moved after a SHOW click. Stamped
    // once at init too: the boot setView/fitBounds settled before this
    // listener existed.
    const stampCenter = () => {
      const w = fromLatLng(map.getCenter());
      const rootEl = map.getContainer().closest<HTMLElement>('[data-testid="map-root"]');
      (rootEl ?? map.getContainer()).dataset.center = `${w.x.toFixed(0)},${w.y.toFixed(0)}`;
    };
    // The eased zoom fires `moveend` once per frame; debounce the EXPENSIVE work
    // (a backend persistence write + the React % readout) to the true settle so a
    // ~1s gesture is one write + one re-render, not ~60. The cheap DOM stamps
    // above run inline so the smoothness/testability signal stays per-frame.
    let settleTimer: ReturnType<typeof setTimeout> | null = null;
    const persistSettle = () => {
      if (settleTimer) clearTimeout(settleTimer);
      settleTimer = setTimeout(() => {
        settleTimer = null;
        const c = map.getCenter();
        // Principle 1: position is never lost — persisted on every settle.
        useStore.getState().saveViewState({ map: { center: [c.lat, c.lng], zoom: map.getZoom() } });
        setZoomPct(Math.round(Math.pow(2, map.getZoom() - 2) * 100));
      }, 200);
    };
    map.on("moveend", () => {
      stampCenter();
      persistSettle();
    });
    stampCenter();

    // Eased, continuous wheel zoom (replaces Leaflet's chunky stepped zoom).
    const detachZoom = attachSmoothWheelZoom(map, map.getContainer());

    return () => {
      detachZoom();
      if (settleTimer) clearTimeout(settleTimer);
      map.remove();
      mapRef.current = null;
      layerRef.current = null;
      markersRef.current.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ---- MANIFOLD map motion (§5: 7a accept sweep, 7b placement drop, 7c
  // import cluster converge, 7e route draw, map half of 7h). The detector
  // diffs plan entities between commits; the store's hash-pinned verb picks
  // the grammar (accepts/imports don't stamp one — they're recognized by the
  // reviewing transition / verb-less built additions). Skipped wholesale
  // under prefers-reduced-motion. ----
  const reducedMotion =
    typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  // fresh pin id → mount class + delay, consumed by the marker-sync effect
  const pinMotionRef = useRef<Map<string, { cls: string; delayMs: number }>>(new Map());
  const mapMotionRef = useRef<MapMotion | null>(null);
  const [motionTick, setMotionTick] = useState(0);
  const [acceptChip, setAcceptChip] = useState<{ count: number; at: number; delayMs: number } | null>(null);
  const [pinGhosts, setPinGhosts] = useState<{ id: string; left: number; top: number }[]>([]);
  const prevPlanRef = useRef<{
    factories: Set<string>;
    claims: Set<string>;
    routes: Set<string>;
    reviewing: boolean;
  } | null>(null);
  const ghostTimerRef = useRef<number | null>(null);

  // Spec State-Management note: 7a/7c sequences "interrupt cleanly if the
  // user interacts (jump to end state)". Flushing = strip every pin's motion
  // class (the natural render IS the end state), drop queued classes, end
  // the canvas window, repaint static.
  const flushMapMotion = useCallback(() => {
    pinMotionRef.current.clear();
    for (const marker of markersRef.current.values()) {
      const el = marker.getElement();
      if (!el) continue;
      el.classList.remove("pin-motion-drop", "pin-motion-land", "pin-motion-cluster", "pin-motion-pop");
      el.style.removeProperty("--pin-delay");
    }
    if (mapMotionRef.current) {
      mapMotionRef.current = null;
      setMotionTick((n) => n + 1);
    }
  }, []);
  // Arm the interrupt only while a sequenced window is live — the triggering
  // interaction itself never flushes (listeners attach after the commit).
  useEffect(() => {
    const m = mapMotionRef.current;
    const map = mapRef.current;
    if (!m || !map || Date.now() >= m.until) return;
    const flush = () => flushMapMotion();
    map.on("movestart zoomstart mousedown", flush);
    window.addEventListener("keydown", flush, true);
    return () => {
      map.off("movestart zoomstart mousedown", flush);
      window.removeEventListener("keydown", flush, true);
    };
  }, [motionTick, flushMapMotion]);

  useEffect(() => {
    const cur = {
      factories: new Set(Object.keys(plan.factories)),
      claims: new Set(Object.keys(plan.nodeClaims)),
      routes: new Set(Object.keys(plan.routes)),
      reviewing: !!reviewingProposal,
    };
    const prev = prevPlanRef.current;
    prevPlanRef.current = cur;
    // First commit (boot hydrate) is never a mutation.
    if (!prev || reducedMotion) return;
    const now = Date.now();
    const st = useStore.getState();
    const fAdd = [...cur.factories].filter((id) => !prev.factories.has(id));
    const fRem = [...prev.factories].filter((id) => !cur.factories.has(id));
    const cAdd = [...cur.claims].filter((id) => !prev.claims.has(id));
    const rAdd = [...cur.routes].filter((id) => !prev.routes.has(id));
    if (!fAdd.length && !fRem.length && !cAdd.length && !rAdd.length) return;
    const verb = motionKind(st.motion, now, st.planHash);
    const claimTethers = (ids: string[], offsetMs: number) => {
      const born: Record<string, number> = {};
      for (const id of ids) {
        const n = world.nodes.find((w) => w.id === plan.nodeClaims[id]?.node);
        if (n) born[tetherKey(n)] = now + offsetMs;
      }
      return born;
    };
    // A later mutation MERGES into a still-live window instead of replacing
    // it — an in-flight draw-in is never truncated by the next edit.
    const mergeMotion = (m: MapMotion) => {
      const live = mapMotionRef.current && now < mapMotionRef.current.until ? mapMotionRef.current : null;
      mapMotionRef.current = live
        ? {
            until: Math.max(live.until, m.until),
            tetherBorn: { ...live.tetherBorn, ...m.tetherBorn },
            routeBorn: { ...live.routeBorn, ...m.routeBorn },
            clusters: [...live.clusters, ...m.clusters],
          }
        : m;
      setMotionTick((n) => n + 1);
    };
    if (prev.reviewing && !cur.reviewing && (fAdd.length || cAdd.length || rAdd.length)) {
      // 7a — accepted rows sweep onto the map: pins land in sequence (40ms
      // stagger, left → right), tethers/routes draw in, the summary chip
      // rises last. Exactly one undo step — acceptProposal is one command.
      const sorted = fAdd.map((id) => plan.factories[id]).sort((a, b) => a.position.x - b.position.x);
      sorted.forEach((f, i) => pinMotionRef.current.set(f.id, { cls: "pin-motion-land", delayMs: i * 40 }));
      mergeMotion({
        until: now + 1200 + sorted.length * 40,
        tetherBorn: claimTethers(cAdd, 250),
        routeBorn: Object.fromEntries(rAdd.map((id) => [id, now + 300])),
        clusters: [],
      });
      // the chip rises LAST — after the pin sweep and the tether draw
      setAcceptChip({
        count: fAdd.length + cAdd.length + rAdd.length,
        at: now,
        delayMs: sorted.length * 40 + 300 + 200,
      });
    } else if (verb === "edit") {
      // 7b — a placed pin drops in; 7e — a bound route draws A → B; a fresh
      // claim's tether draws node → factory.
      for (const id of fAdd) pinMotionRef.current.set(id, { cls: "pin-motion-drop", delayMs: 0 });
      if (rAdd.length || cAdd.length) {
        mergeMotion({
          until: now + 700,
          tetherBorn: claimTethers(cAdd, 0),
          routeBorn: Object.fromEntries(rAdd.map((id) => [id, now])),
          clusters: [],
        });
      }
    } else if (verb === "undo" || verb === "redo") {
      // 7h — a returning pin pops 1.12×; an undo-removed pin leaves a dashed
      // ghost where it stood (position read from the still-mounted marker —
      // the marker-sync effect below runs after this one).
      for (const id of fAdd) pinMotionRef.current.set(id, { cls: "pin-motion-pop", delayMs: 0 });
      if (verb === "undo" && fRem.length) {
        const map = mapRef.current;
        const gs = fRem.flatMap((id) => {
          const m = markersRef.current.get(id);
          if (!m || !map) return [];
          const p = map.latLngToContainerPoint(m.getLatLng());
          return [{ id, left: p.x, top: p.y }];
        });
        if (gs.length) {
          setPinGhosts(gs);
          // rapid successive undos must not cut a live ghost short
          if (ghostTimerRef.current) window.clearTimeout(ghostTimerRef.current);
          ghostTimerRef.current = window.setTimeout(() => setPinGhosts([]), 300);
        }
      }
    } else if (!verb) {
      // 7c — verb-less BUILT additions only ever come from a save import:
      // per cluster, machine dots converge on the ◆ centroid, the pin pops,
      // clusters play sequentially left → right (capped — a giant import
      // animates its first CLUSTER_CAP clusters and the rest just appear).
      const sorted = fAdd
        .map((id) => plan.factories[id])
        .filter((f) => f && f.status === "built")
        .sort((a, b) => a.position.x - b.position.x)
        .slice(0, CLUSTER_CAP);
      if (!sorted.length) return;
      // A background auto-pull the user never initiated gets the QUICK land
      // sweep, not the multi-second cluster cinematic — that belongs to
      // imports the user is watching for.
      if (st.syncAppliedAt && now - st.syncAppliedAt < 5000) {
        sorted.forEach((f, i) => pinMotionRef.current.set(f.id, { cls: "pin-motion-land", delayMs: i * 40 }));
        mergeMotion({ until: now + 800 + sorted.length * 40, tetherBorn: {}, routeBorn: {}, clusters: [] });
        return;
      }
      sorted.forEach((f, i) =>
        pinMotionRef.current.set(f.id, {
          cls: "pin-motion-cluster",
          delayMs: i * CLUSTER_STEP_MS + CONVERGE_MS - 150,
        }),
      );
      mergeMotion({
        until: now + (sorted.length - 1) * CLUSTER_STEP_MS + CONVERGE_MS + 400,
        tetherBorn: {},
        routeBorn: {},
        clusters: sorted.map((f, i) => ({
          x: f.position.x,
          y: f.position.y,
          dots: scatter(f.id, Math.max(4, Math.min(12, f.groups.length)), 110),
          startAt: now + i * CLUSTER_STEP_MS,
        })),
      });
    }
  }, [plan.factories, plan.nodeClaims, plan.routes, reviewingProposal, world.nodes, reducedMotion, plan]);

  // The accept chip clears itself after its delayed rise + hold.
  useEffect(() => {
    if (!acceptChip) return;
    const t = window.setTimeout(() => setAcceptChip(null), 2600 + acceptChip.delayMs);
    return () => window.clearTimeout(t);
  }, [acceptChip]);
  // Ghost timer hygiene on unmount.
  useEffect(
    () => () => {
      if (ghostTimerRef.current) window.clearTimeout(ghostTimerRef.current);
    },
    [],
  );

  // ---- canvas layer data sync ----
  useEffect(() => {
    // Lane assignment: routes (cargo AND power) between the same factory
    // pair fan out with stable perpendicular offsets instead of stacking.
    const laneOf = new Map<string, { lane: number; lanes: number }>();
    {
      const groups = new Map<string, string[]>();
      // cargo endpoints are PORT ids, power endpoints are FACTORY ids —
      // normalize to factories so mixed kinds share the fan
      const owner = (e: string) => plan.ports[e]?.factory ?? e;
      for (const r of Object.values(plan.routes)) {
        const key = r.endpoints.map(owner).sort().join("|");
        groups.set(key, [...(groups.get(key) ?? []), r.id]);
      }
      for (const ids of groups.values()) {
        ids.sort();
        ids.forEach((id, i) => laneOf.set(id, { lane: i, lanes: ids.length }));
      }
    }
    const routes = Object.values(plan.routes)
      .filter((r) => CARGO_KINDS.has(r.kind.kind))
      .map((r) => {
        const d = derived.routes[r.id];
        const itemClass = r.manifest[0]?.[0] ?? "";
        return {
          id: r.id,
          path: r.path,
          planned: r.status === "planned",
          saturation: d?.saturation ?? 0,
          flow: d?.flow ?? 0,
          capacity: d?.capacity ?? 0,
          // honest red only: a deficit through this route while it runs full
          bottleneck: routeBottleneck(r.id, d?.saturation ?? 0, derived.deficits),
          kind: r.kind.kind as "belt" | "rail" | "truck" | "drone",
          tag: r.kind.kind === "belt" ? `MK.${r.kind.tier}` : r.kind.kind.toUpperCase(),
          itemName: itemLabel(gamedata.items, itemClass).toUpperCase(),
          selected: selection?.kind === "route" && selection.id === r.id,
          ...(laneOf.get(r.id) ?? { lane: 0, lanes: 1 }),
        };
      });
    // power lines connect factory pins; the chip carries the grid margin
    const powerLines = Object.values(plan.routes)
      .filter((r) => r.kind.kind === "power")
      .map((r) => ({
        id: r.id,
        ...(laneOf.get(r.id) ?? { lane: 0, lanes: 1 }),
        from: plan.factories[r.endpoints[0]]?.position ?? r.path[0] ?? { x: 0, y: 0 },
        to: plan.factories[r.endpoints[1]]?.position ?? r.path[r.path.length - 1] ?? { x: 0, y: 0 },
        selected: selection?.kind === "route" && selection.id === r.id,
      }));
    const shedBySwitch: Record<string, number> = {};
    for (const c of derived.circuits) for (const sw of c.switches) shedBySwitch[sw.id] = sw.shedsAtMw;
    const switches = Object.values(plan.switches).map((sw) => ({
      id: sw.id,
      x: sw.position.x,
      y: sw.position.y,
      priority: sw.priority,
      chip:
        shedBySwitch[sw.id] != null ? `P${sw.priority} · SHEDS AT ${fmtPower(shedBySwitch[sw.id])}` : `P${sw.priority}`,
      selected: selection?.kind === "switch" && selection.id === sw.id,
    }));
    const circuitChips = derived.circuits
      .map((c) => {
        const pts = c.members.map((m) => plan.factories[m]?.position).filter((p): p is { x: number; y: number } => !!p);
        if (!pts.length) return null;
        return {
          x: pts.reduce((s, p) => s + p.x, 0) / pts.length,
          y: pts.reduce((s, p) => s + p.y, 0) / pts.length,
          text: `${c.name} · ${fmtPower(c.demandMw)} / ${fmtPower(c.generationMw)}`,
          level: circuitLevel(c.generationMw, c.demandMw),
        };
      })
      .filter((c): c is NonNullable<typeof c> => c !== null);
    // proposal review ghosts: parse included items' commands (mock 3a grammar)
    let review: NonNullable<Parameters<MapCanvasLayer["setData"]>[0]["review"]> | null = null;
    if (reviewingProposal) {
      review = { pins: [], claimRings: [], modifyRings: [], lines: [] };
      for (const item of reviewingProposal.items) {
        if (!item.included) continue;
        for (const cmd of item.commands) {
          if (cmd.type === "create_factory") {
            review.pins.push({ x: cmd.position.x, y: cmd.position.y, name: cmd.name });
          } else if (cmd.type === "claim_node") {
            const n = world.nodes.find((w) => w.id === cmd.node);
            if (n) review.claimRings.push({ x: n.x, y: n.y });
          } else if (cmd.type === "add_route" && cmd.path.length >= 2) {
            review.lines.push({
              from: cmd.path[0],
              to: cmd.path[cmd.path.length - 1],
              power: cmd.kind.kind === "power",
            });
          } else if (cmd.type === "set_port_rate" && !cmd.id.startsWith("$")) {
            const port = plan.ports[cmd.id];
            const f = port ? plan.factories[port.factory] : null;
            if (f) review.modifyRings.push({ x: f.position.x, y: f.position.y });
          } else if (cmd.type === "set_group_recipe" && !cmd.id.startsWith("$")) {
            const g = plan.groups[cmd.id];
            const f = g ? plan.factories[g.factory] : null;
            if (f) review.modifyRings.push({ x: f.position.x, y: f.position.y });
          }
        }
      }
    }
    const src = routeDraft ? plan.factories[routeDraft.from] : null;
    layerRef.current?.setData({
      world: resolvedWorld,
      nodeStates,
      claimLinks,
      replacesLinks,
      hoveredNode: hoveredNode?.id ?? null,
      selectedNode: selection?.kind === "node" ? selection.id : null,
      showNodes: overlays.nodes,
      nodeFilter,
      showTerrain: overlays.terrain,
      routes,
      showRoutes: overlays.flows,
      powerLines,
      circuitChips,
      switches,
      showPower: overlays.power,
      ghost: src && routeDraft ? { from: src.position, to: routeDraft.cursor } : null,
      review,
      motion: mapMotionRef.current && Date.now() < mapMotionRef.current.until ? mapMotionRef.current : null,
    });
  }, [resolvedWorld, nodeStates, claimLinks, replacesLinks, hoveredNode, selection, overlays, nodeFilter, plan, derived.routes, derived.circuits, gamedata.items, routeDraft, reviewingProposal, motionTick]);

  // ---- pointer interactions (hover + click on canvas nodes, placement) ----
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    // One arbitration rule for hover AND click, so the cursor never promises
    // a different target than a click selects. Nodes keep their 12px comfort
    // zone in open space, but a line passing through the dense real-node
    // field must stay clickable: within 8px of a dot the node wins (you're ON
    // it), farther out a route/switch hit takes precedence.
    const resolveHit = (pt: L.Point) => {
      const layer = layerRef.current;
      const nodeHit = layer?.hitTestNode(pt) ?? null;
      const switchHit = layer?.hitTestSwitch(pt) ?? null;
      const routeHit = layer?.hitTestRoute(pt) ?? layer?.hitTestPower(pt) ?? null;
      const nodeWins = nodeHit && (nodeHit.d <= 8 || (!switchHit && !routeHit));
      return {
        node: nodeWins ? nodeHit.node : null,
        switchHit: nodeWins ? null : switchHit,
        routeHit: nodeWins || switchHit ? null : routeHit,
      };
    };
    const onMove = (e: L.LeafletMouseEvent) => {
      if (routeDraftRef.current) {
        setRouteDraft({ from: routeDraftRef.current.from, cursor: fromLatLng(e.latlng) });
        return;
      }
      const pt = map.latLngToContainerPoint(e.latlng);
      const { node: hit, switchHit, routeHit } = resolveHit(pt);
      setHoveredNode(hit);
      map.getContainer().style.cursor = placing ? "crosshair" : hit || switchHit || routeHit ? "pointer" : "";
    };
    const onClick = (e: L.LeafletMouseEvent) => {
      if (placing) {
        const pos = fromLatLng(e.latlng);
        const region =
          [...world.regions].sort(
            (a, b) =>
              Math.hypot(a.labelX - pos.x, a.labelY - pos.y) - Math.hypot(b.labelX - pos.x, b.labelY - pos.y),
          )[0]?.name ?? "";
        const n = Object.keys(useStore.getState().plan.factories).length + 1;
        void dispatch(
          [{ type: "create_factory", name: `FACTORY ${n}`, position: pos, region }],
          { select: true },
        );
        setPlacing(false);
        return;
      }
      const pt = map.latLngToContainerPoint(e.latlng);
      const { node: hit, switchHit, routeHit } = resolveHit(pt);
      if (hit) setSelection({ kind: "node", id: hit.id });
      else if (switchHit) setSelection({ kind: "switch", id: switchHit });
      else if (routeHit) setSelection({ kind: "route", id: routeHit });
      else setSelection(null);
    };
    // right-drag from a pin draws a route (ghost-blue until confirmed).
    // mouseup listens on window so releasing over a drawer or other chrome
    // still ends the drag instead of leaving a stuck ghost line.
    const onMouseUp = (e: MouseEvent) => {
      const draft = routeDraftRef.current;
      if (!draft || e.button !== 2) return;
      // Chromium fires contextmenu after the right-button release; suppress
      // that one (and only that one) even when it lands outside the map.
      suppressCtxRef.current = true;
      const st = useStore.getState();
      const rect = map.getContainer().getBoundingClientRect();
      const pt = L.point(e.clientX - rect.left, e.clientY - rect.top);
      let target: string | null = null;
      for (const f of Object.values(st.plan.factories)) {
        const fp = map.latLngToContainerPoint(toLatLng(f.position));
        if (f.id !== draft.from && Math.hypot(fp.x - pt.x, fp.y - pt.y) < 28) target = f.id;
      }
      setRouteDraft(null);
      if (target) setRoutePopover({ from: draft.from, to: target });
    };
    const onCtx = (e: Event) => e.preventDefault();
    const onWindowCtx = (e: Event) => {
      if (suppressCtxRef.current) {
        suppressCtxRef.current = false;
        e.preventDefault();
      }
    };
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("contextmenu", onWindowCtx);
    map.getContainer().addEventListener("contextmenu", onCtx);
    map.on("mousemove", onMove);
    map.on("click", onClick);
    return () => {
      map.off("mousemove", onMove);
      map.off("click", onClick);
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("contextmenu", onWindowCtx);
      map.getContainer().removeEventListener("contextmenu", onCtx);
    };
  }, [placing, world.regions, dispatch, setPlacing, setSelection]);

  // ---- factory pin markers ----
  // Firefox (notably on Linux) eagerly starts a NATIVE HTML drag from the
  // pin's inline svg / label text. Once a native drag begins, the mouseup is
  // swallowed, Leaflet's marker Draggable never gets its dragend, and the pin
  // LATCHES to the pointer — every later map pan also "drags the factory",
  // committing bogus positions. Kill native dragstart at the source (paired
  // with user-select:none on .pin-icon in map.css). DivIcon.createIcon reuses
  // the existing outer DIV on setIcon, so the guard persists for the marker's
  // whole life; the WeakSet keeps the re-arm-after-setIcon insurance (for a
  // future icon-type swap that DOES replace the element) from stacking
  // duplicate listeners on the reused element.
  const armedPinsRef = useRef(new WeakSet<HTMLElement>());
  const armPinElement = (marker: L.Marker) => {
    const el = marker.getElement();
    if (!el || armedPinsRef.current.has(el)) return;
    armedPinsRef.current.add(el);
    el.addEventListener("dragstart", (e) => e.preventDefault());
  };
  // Marker drags whose release the safety net had to force are discarded —
  // suppress the dragend commit and snap back to the stored position.
  const discardDragRef = useRef(new Set<string>());
  const pinHtmlRef = useRef(new Map<string, string>());
  // Bumped when the safety net rescues a drag: a discarded drag dispatches
  // nothing, so without this the marker-sync effect (which skips a pin while
  // it is mid-drag) would never re-run to repaint icon/draggable state that
  // changed underneath the stuck drag.
  const [rescueTick, setRescueTick] = useState(0);

  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    const markers = markersRef.current;
    const seen = new Set<string>();
    // W2a cutover tags: a factory named by some `replaces` is RETIRING; a
    // factory that carries `replaces` is the INCOMING replacement.
    const retiring = new Set<string>();
    for (const f of Object.values(plan.factories)) if (f.replaces) retiring.add(f.replaces);
    for (const f of Object.values(plan.factories)) {
      seen.add(f.id);
      const selected = selection?.kind === "factory" && selection.id === f.id;
      const tag = f.replaces ? "incoming" : retiring.has(f.id) ? "retiring" : undefined;
      const html = pinHtml(f.name, f.status, selected, tag);
      const icon = L.divIcon({ className: "pin-icon", html, iconSize: [30, 30], iconAnchor: [15, 15] });
      let marker = markers.get(f.id);
      if (!marker) {
        marker = L.marker(toLatLng(f.position), {
          icon,
          draggable: f.status === "planned",
          keyboard: false,
        });
        marker.on("click", () => useStore.getState().setSelection({ kind: "factory", id: f.id }));
        marker.on("dblclick", () => useStore.getState().setView({ mode: "factory", factoryId: f.id }));
        marker.on("mousedown", (ev: L.LeafletMouseEvent) => {
          if ((ev.originalEvent as MouseEvent).button === 2) {
            L.DomEvent.stop(ev);
            setRouteDraft({ from: f.id, cursor: fromLatLng(ev.latlng) });
          }
        });
        marker.on("dragend", () => {
          // a force-released (stuck) drag is not a user drop — discard it
          if (discardDragRef.current.delete(f.id)) return;
          // keep the planner-entered elevation — dragging only moves x/y
          const z = useStore.getState().plan.factories[f.id]?.position.z ?? 0;
          const pos = { ...fromLatLng(marker!.getLatLng()), z };
          void useStore.getState().dispatch([{ type: "move_factory_pin", id: f.id, position: pos }]);
        });
        marker.addTo(map);
        armPinElement(marker);
        // MANIFOLD motion: a fresh pin the detector tagged plays its mount
        // grammar (7a land / 7b drop / 7c cluster pop / 7h return pop).
        const mm = pinMotionRef.current.get(f.id);
        if (mm) {
          pinMotionRef.current.delete(f.id);
          const el = marker.getElement();
          if (el) {
            el.style.setProperty("--pin-delay", `${mm.delayMs}ms`);
            el.classList.add(mm.cls);
          }
        }
        pinHtmlRef.current.set(f.id, html);
        markers.set(f.id, marker);
      } else if (!(marker.dragging as unknown as { _draggable?: { _moving?: boolean } })?._draggable?._moving) {
        // Never mutate a pin MID-DRAG: setIcon() replaces the exact DOM element
        // Leaflet's Draggable is bound to and the enable/disable toggle resets
        // its state — either strands the drag so dragend (and the map-drag
        // re-enable) never lands. The post-drag effect re-syncs everything.
        if (pinHtmlRef.current.get(f.id) !== html) {
          marker.setIcon(icon); // replaces the element — re-arm the native-drag guard
          armPinElement(marker);
          pinHtmlRef.current.set(f.id, html);
        }
        // re-sync draggability: a pin that flips ◇→◈→◆ (or back on undo) must
        // gain/lose its drag handle, not keep the value it was created with.
        const draggable = f.status === "planned";
        marker.options.draggable = draggable;
        if (marker.dragging) marker.dragging[draggable ? "enable" : "disable"]();
        const ll = toLatLng(f.position) as [number, number];
        const cur = marker.getLatLng();
        if (Math.abs(cur.lat - ll[0]) > 1e-9 || Math.abs(cur.lng - ll[1]) > 1e-9) marker.setLatLng(ll);
      }
    }
    for (const [id, marker] of markers) {
      if (!seen.has(id)) {
        marker.remove();
        markers.delete(id);
        pinHtmlRef.current.delete(id);
        discardDragRef.current.delete(id);
      }
    }
    declutterPinChips(map, markers);
  }, [plan.factories, selection, rescueTick]);

  // Stuck-drag safety net: if a pin's Draggable still thinks it is moving when
  // the interaction has plainly ended, its release was lost. Force-finish it
  // and restore the stored position — a drag that needed rescuing was never an
  // intentional drop. Channels, by failure mode:
  //  - "dragend": a NATIVE drag swallowed the mouseup (the Firefox bug) — no
  //    mouseup will ever arrive, but the native drag's own dragend does.
  //  - mousemove with buttons===0: the release happened OFF-window (X11), so
  //    no mouseup reaches the document at all; a button-less move while a
  //    Draggable is mid-flight is proof the drag already ended. During a real
  //    drag buttons!==0, so this can never discard a legitimate drop.
  //  - "blur": focus stolen mid-drag.
  //  - window "mouseup": belt-and-suspenders; for mouse-origin drags Leaflet's
  //    own DOCUMENT-level handler runs first (bubble order) and finishes the
  //    drag before this fires — deliberately so, since pre-empting it (capture
  //    phase) would discard every normal drop. It still covers touch-origin
  //    latches, whose Leaflet listeners are touchend-based.
  useEffect(() => {
    const release = () => {
      let rescued = false;
      for (const [id, m] of markersRef.current) {
        const d = (m.dragging as unknown as { _draggable?: { _moving?: boolean; finishDrag?: () => void } })?._draggable;
        if (d?._moving && typeof d.finishDrag === "function") {
          discardDragRef.current.add(id);
          try {
            d.finishDrag();
          } finally {
            // When finishDrag fired dragend (synchronously), the handler
            // already consumed the flag and this is a no-op. When it did NOT
            // (re-mousedown on a latched pin resets _moved, so Leaflet skips
            // dragend), this cleans the flag — left stale it would silently
            // swallow the NEXT legitimate drop of this pin.
            discardDragRef.current.delete(id);
          }
          const f = useStore.getState().plan.factories[id];
          if (f) m.setLatLng(toLatLng(f.position));
          rescued = true;
        }
      }
      // A discarded drag dispatches nothing, so nothing else re-runs the
      // marker sync — bump it to repaint icon/draggable state that changed
      // underneath the stuck drag.
      if (rescued) setRescueTick((t) => t + 1);
    };
    const onIdleMove = (e: MouseEvent) => {
      if (e.buttons === 0) release();
    };
    window.addEventListener("mouseup", release);
    window.addEventListener("dragend", release);
    window.addEventListener("blur", release);
    window.addEventListener("mousemove", onIdleMove);
    return () => {
      window.removeEventListener("mouseup", release);
      window.removeEventListener("dragend", release);
      window.removeEventListener("blur", release);
      window.removeEventListener("mousemove", onIdleMove);
    };
  }, []);

  // ---- keys: N place, F frame, ESC deselect, 1/4 overlays, ⏎ dive ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e)) return;
      const map = mapRef.current;
      if (e.key === "n" || e.key === "N") setPlacing(!placing);
      else if (e.key === "p" || e.key === "P") setWizard({ open: true });
      else if (e.key === "Escape") {
        // an in-flight route draft cancels first (via the ref: no new deps);
        // the DATA dropdown closes itself (shell/DataMenu.tsx capture listener)
        if (routeDraftRef.current) setRouteDraft(null);
        else if (placing) setPlacing(false);
        else setSelection(null);
      } else if (e.key === "1") setOverlay("flows", !overlays.flows);
      else if (e.key === "2") setOverlay("power", !overlays.power);
      else if (e.key === "3") setOverlay("nodes", !overlays.nodes);
      else if (e.key === "4") setOverlay("terrain", !overlays.terrain);
      else if (e.key === "f" || e.key === "F") {
        const pts = Object.values(plan.factories).map((f) => toLatLng(f.position));
        if (pts.length && map) map.fitBounds(L.latLngBounds(pts as L.LatLngExpression[]).pad(0.4));
      } else if (e.key === "Enter" && selection?.kind === "factory") {
        setView({ mode: "factory", factoryId: selection.id });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [placing, overlays, plan.factories, selection, setOverlay, setPlacing, setSelection, setView, setWizard]);

  const panTo = useCallback((pos: { x: number; y: number }) => {
    const map = mapRef.current;
    if (!map) return;
    // At the boot zoom the whole world fits the viewport and a plain panTo is
    // a no-op (the center is pinned by maxBounds) — a search jump must also
    // come DOWN to a zoom where centering on the target is possible.
    map.flyTo(toLatLng(pos), Math.max(map.getZoom(), 4));
  }, []);

  // PR 9 flyTo: consume-and-clear, mirroring the auditRequest idiom. Runs on
  // mount too, so a SHOW clicked from graph view still lands — setView swaps
  // in MapView first, then this effect pans once the map exists.
  const flyTo = useStore((s) => s.flyTo);
  const clearFly = useStore((s) => s.clearFly);
  useEffect(() => {
    if (!flyTo) return;
    panTo(flyTo);
    clearFly();
  }, [flyTo, panTo, clearFly]);

  const zoomBy = (d: number) => mapRef.current?.setZoom((mapRef.current?.getZoom() ?? 2) + d);

  const selectedFactory = selection?.kind === "factory" ? plan.factories[selection.id] : null;
  const selectedNode = selection?.kind === "node" ? resolvedNodes.find((n) => n.id === selection.id) : null;

  return (
    <div
      className={`map-root ${reviewing ? "reviewing" : ""}`}
      data-testid="map-root"
      // How many resource nodes the canvas is currently drawing — all of them,
      // or just the search-filtered subset. A testable stamp for the live filter
      // (nodes render to canvas, not the DOM).
      data-nodes-shown={nodeFilter?.active ? nodeFilter.visible.size : resolvedNodes.length}
      onDragEnter={(e) => {
        if (!Array.from(e.dataTransfer?.types ?? []).includes("Files")) return;
        e.preventDefault();
        dragDepth.current += 1;
        setDragging(true);
      }}
      onDragOver={(e) => {
        if (dragging) e.preventDefault();
      }}
      onDragLeave={() => {
        dragDepth.current = Math.max(0, dragDepth.current - 1);
        if (dragDepth.current === 0) setDragging(false);
      }}
      onDrop={(e) => {
        e.preventDefault();
        dragDepth.current = 0;
        setDragging(false);
        const files = Array.from(e.dataTransfer?.files ?? []);
        if (files.length) acceptFiles(files);
      }}
    >
      <div ref={containerRef} className="map-leaflet" />
      {dragging && (
        <div className="map-drop-overlay" data-testid="map-drop-overlay">
          <div className="map-drop-card mono">
            <div className="map-drop-title">DROP TO LOAD</div>
            <div className="map-drop-sub">
              .sav imports your factories{__WASM_BACKEND__ ? " · .json loads your recipe catalog" : ""}
            </div>
          </div>
        </div>
      )}

      {/* top chrome — search docks centered in the titlebar (portal; hidden
          during proposal review, when the map is a read-only preview); chips +
          actions stay over the map */}
      {reviewing ? null : searchSlot ? createPortal(<SearchBox onJump={panTo} />, searchSlot) : <SearchBox onJump={panTo} />}
      <div className="map-chrome-top">
        <div className="map-overlay-chips">
          <button
            className={`btn btn-ghost overlay-chip ${overlays.flows ? "active" : ""}`}
            onClick={() => setOverlay("flows", !overlays.flows)}
            title="Item flow routes (1)"
          >
            FLOWS <span className="key-hint">1</span>
          </button>
          <button
            className={`btn btn-ghost overlay-chip ${overlays.power ? "active" : ""}`}
            onClick={() => setOverlay("power", !overlays.power)}
            title="Power grid (2)"
          >
            POWER <span className="key-hint">2</span>
          </button>
          <button
            className={`btn btn-ghost overlay-chip ${overlays.nodes ? "active" : ""}`}
            onClick={() => setOverlay("nodes", !overlays.nodes)}
            title="Resource nodes (3)"
          >
            NODES <span className="key-hint">3</span>
          </button>
          <button
            className={`btn btn-ghost overlay-chip ${overlays.terrain ? "active" : ""}`}
            onClick={() => setOverlay("terrain", !overlays.terrain)}
            title="Terrain (4)"
            data-testid="btn-overlay-terrain"
          >
            TERRAIN <span className="key-hint">4</span>
          </button>
        </div>
        <div className="map-actions">
          <button
            className={`btn btn-ghost ${placing ? "active" : ""}`}
            onClick={() => setPlacing(!placing)}
            data-testid="btn-add-factory"
          >
            + FACTORY <span className="key-hint">N</span>
          </button>
          <button className="btn btn-primary" onClick={() => setWizard({ open: true })} data-testid="btn-wizard">
            <Glyph name="wizard" size={14} /> PLAN SUPPLY CHAIN <span className="key-hint">P</span>
          </button>
          <div className="zoom-ctl mono">
            <button onClick={() => zoomBy(-0.5)} aria-label="Zoom out">
              −
            </button>
            <span data-testid="zoom-pct">{zoomPct}%</span>
            <button onClick={() => zoomBy(0.5)} aria-label="Zoom in">
              +
            </button>
          </div>
        </div>
      </div>

      {placing && (
        <div className="map-placing-hint mono">CLICK TO PLACE FACTORY — ESC TO CANCEL</div>
      )}

      {hoveredNode && !selectedNode && <NodeTooltip node={hoveredNode} />}

      <ResourceOverview />
      <Legend />

      {selectedFactory && <SummaryDrawer factory={selectedFactory} />}
      {selectedNode && <NodeDrawer node={selectedNode} />}
      {selection?.kind === "route" && plan.routes[selection.id] && (
        <RouteDrawer route={plan.routes[selection.id]} />
      )}
      {selection?.kind === "switch" && plan.switches[selection.id] && (
        <SwitchDrawer sw={plan.switches[selection.id]} />
      )}
      {routePopover && (
        <RoutePopover
          fromFactory={routePopover.from}
          toFactory={routePopover.to}
          onClose={() => setRoutePopover(null)}
        />
      )}
      {routeDraft && <div className="map-placing-hint mono">RELEASE OVER A FACTORY TO BIND THE ROUTE</div>}

      {/* Motion 7a — the accept summary chip rises last (bp grammar). */}
      {acceptChip && (
        <div
          className="map-accept-chip mono"
          style={{ "--chip-delay": `${acceptChip.delayMs}ms` } as CSSProperties}
          data-testid="map-accept-chip"
        >
          +{acceptChip.count} {acceptChip.count === 1 ? "ENTITY" : "ENTITIES"} — 1 UNDO STEP
        </div>
      )}
      {/* Motion 7h — undo-removed pins flash a dashed ghost where they stood. */}
      {pinGhosts.map((g) => (
        <div
          key={g.id}
          className="map-pin-ghost"
          style={{ left: g.left, top: g.top }}
          data-testid={`pin-ghost-${g.id}`}
          aria-hidden
        />
      ))}
    </div>
  );
}

function NodeTooltip({ node }: { node: WorldNode }) {
  const items = useStore((s) => s.gamedata.items);
  // save-only nodes carry item:"" — degrade to a readable label, never blank.
  const name = itemLabel(items, node.item) || "RESOURCE NODE";
  return (
    <div className="node-tooltip chip">
      {name.toUpperCase()} · {(node.purity || "UNKNOWN").toUpperCase()}
      {node.zone === "cave" ? " · ▾CAVE" : ""}
    </div>
  );
}
