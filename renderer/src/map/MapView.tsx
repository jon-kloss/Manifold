// Map home (mock 2a). The map IS the app: everything else is a layer or zoom
// level. Leaflet CRS.Simple; canvas layer for grid/labels/nodes; DOM pins for
// factories; placeholder treatment for world imagery (runtime asset — see
// DECISIONS.md on tile licensing).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { MapCanvasLayer } from "./CanvasLayer";
import type { CanvasLayerData, NodeRenderState } from "./CanvasLayer";
import { fromLatLng, toLatLng } from "./maputil";
import { useStore } from "../state/store";
import { isEditableTarget } from "../lib/keys";
import type { WorldNode } from "../state/types";
import SummaryDrawer from "./SummaryDrawer";
import NodeDrawer from "./NodeDrawer";
import RouteDrawer from "./RouteDrawer";
import SwitchDrawer from "./SwitchDrawer";
import RoutePopover from "./RoutePopover";
import Legend from "./Legend";
import SearchBox from "./SearchBox";
import ImportModal from "../import/ImportModal";
import { fmtPower, prettyClass } from "../lib/format";
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
      const w = f.name.length * 6.4 + 34; // 10px mono + glyph + padding
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
  const [importFile, setImportFile] = useState<File | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
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

  const claimLinks = useMemo(() => {
    const nodeById: Record<string, { x: number; y: number }> = {};
    for (const n of resolvedNodes) nodeById[n.id] = { x: n.x, y: n.y };
    const links: CanvasLayerData["claimLinks"] = [];
    for (const c of Object.values(plan.nodeClaims)) {
      const node = nodeById[c.node];
      const f = plan.factories[c.factory];
      if (!node || !f) continue;
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
  }, [plan.nodeClaims, plan.factories, resolvedNodes, derived.nodes, selection, hoveredNode]);

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
      zoomSnap: 0.5,
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
      showTerrain: useStore.getState().overlays.terrain,
      routes: [],
      showRoutes: true,
      powerLines: [],
      circuitChips: [],
      switches: [],
      showPower: true,
      ghost: null,
      review: null,
    });
    layer.addTo(map);
    layerRef.current = layer;
    map.on("move zoom viewreset", () => declutterPinChips(map, markersRef.current));
    mapRef.current = map;
    setZoomPct(Math.round(Math.pow(2, map.getZoom() - 2) * 100));

    map.on("zoomend", () => setZoomPct(Math.round(Math.pow(2, map.getZoom() - 2) * 100)));
    map.on("moveend", () => {
      const c = map.getCenter();
      // Principle 1: position is never lost — persisted on every settle.
      useStore.getState().saveViewState({ map: { center: [c.lat, c.lng], zoom: map.getZoom() } });
    });

    return () => {
      map.remove();
      mapRef.current = null;
      layerRef.current = null;
      markersRef.current.clear();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

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
          kind: r.kind.kind as "belt" | "rail" | "truck" | "drone",
          tag: r.kind.kind === "belt" ? `MK.${r.kind.tier}` : r.kind.kind.toUpperCase(),
          itemName: (gamedata.items[itemClass]?.displayName ?? prettyClass(itemClass)).toUpperCase(),
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
      showTerrain: overlays.terrain,
      routes,
      showRoutes: overlays.flows,
      powerLines,
      circuitChips,
      switches,
      showPower: overlays.power,
      ghost: src && routeDraft ? { from: src.position, to: routeDraft.cursor } : null,
      review,
    });
  }, [resolvedWorld, nodeStates, claimLinks, replacesLinks, hoveredNode, selection, overlays, plan, derived.routes, derived.circuits, gamedata.items, routeDraft, reviewingProposal]);

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
          // keep the planner-entered elevation — dragging only moves x/y
          const z = useStore.getState().plan.factories[f.id]?.position.z ?? 0;
          const pos = { ...fromLatLng(marker!.getLatLng()), z };
          void useStore.getState().dispatch([{ type: "move_factory_pin", id: f.id, position: pos }]);
        });
        marker.addTo(map);
        markers.set(f.id, marker);
      } else {
        marker.setIcon(icon);
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
      }
    }
    declutterPinChips(map, markers);
  }, [plan.factories, selection]);

  // ---- keys: N place, F frame, ESC deselect, 1/4 overlays, ⏎ dive ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e)) return;
      const map = mapRef.current;
      if (e.key === "n" || e.key === "N") setPlacing(!placing);
      else if (e.key === "p" || e.key === "P") setWizard({ open: true });
      else if (e.key === "Escape") {
        // an in-flight route draft cancels first (via the ref: no new deps)
        if (routeDraftRef.current) setRouteDraft(null);
        else if (placing) setPlacing(false);
        else setSelection(null);
      } else if (e.key === "1") setOverlay("flows", !overlays.flows);
      else if (e.key === "2") setOverlay("power", !overlays.power);
      else if (e.key === "4") setOverlay("nodes", !overlays.nodes);
      else if (e.key === "3") setOverlay("terrain", !overlays.terrain);
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
    mapRef.current?.panTo(toLatLng(pos));
  }, []);

  const zoomBy = (d: number) => mapRef.current?.setZoom((mapRef.current?.getZoom() ?? 2) + d);

  const selectedFactory = selection?.kind === "factory" ? plan.factories[selection.id] : null;
  const selectedNode = selection?.kind === "node" ? resolvedNodes.find((n) => n.id === selection.id) : null;

  return (
    <div className={`map-root ${reviewing ? "reviewing" : ""}`} data-testid="map-root">
      <div ref={containerRef} className="map-leaflet" />

      {/* top chrome */}
      <div className="map-chrome-top">
        <SearchBox onJump={panTo} />
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
            title="Resource nodes (4)"
          >
            NODES <span className="key-hint">4</span>
          </button>
          <button
            className={`btn btn-ghost overlay-chip ${overlays.terrain ? "active" : ""}`}
            onClick={() => setOverlay("terrain", !overlays.terrain)}
            data-testid="btn-overlay-terrain"
          >
            TERRAIN <span className="key-hint">3</span>
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
          <button className="btn btn-ghost" onClick={() => fileRef.current?.click()} data-testid="btn-import">
            IMPORT SAVE
          </button>
          <input
            ref={fileRef}
            type="file"
            accept=".sav"
            style={{ display: "none" }}
            data-testid="import-file-input"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) setImportFile(f);
              e.currentTarget.value = "";
            }}
          />
          <button className="btn btn-primary" onClick={() => setWizard({ open: true })} data-testid="btn-wizard">
            PLAN SUPPLY CHAIN <span className="key-hint">P</span>
          </button>
          <div className="zoom-ctl mono">
            <button onClick={() => zoomBy(-0.5)} aria-label="Zoom out">
              −
            </button>
            <span>{zoomPct}%</span>
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
      {importFile && <ImportModal file={importFile} onClose={() => setImportFile(null)} />}
    </div>
  );
}

function NodeTooltip({ node }: { node: WorldNode }) {
  const items = useStore((s) => s.gamedata.items);
  // save-only nodes carry item:"" — degrade to a readable label, never blank.
  const name = items[node.item]?.displayName ?? (node.item || "RESOURCE NODE");
  return (
    <div className="node-tooltip chip">
      {name.toUpperCase()} · {(node.purity || "UNKNOWN").toUpperCase()}
      {node.zone === "cave" ? " · ▾CAVE" : ""}
    </div>
  );
}
