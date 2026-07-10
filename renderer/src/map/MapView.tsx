// Map home (mock 2a). The map IS the app: everything else is a layer or zoom
// level. Leaflet CRS.Simple; canvas layer for grid/labels/nodes; DOM pins for
// factories; placeholder treatment for world imagery (runtime asset — see
// DECISIONS.md on tile licensing).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import { MapCanvasLayer } from "./CanvasLayer";
import type { NodeRenderState } from "./CanvasLayer";
import { fromLatLng, toLatLng } from "./maputil";
import { useStore } from "../state/store";
import type { WorldNode } from "../state/types";
import SummaryDrawer from "./SummaryDrawer";
import NodeDrawer from "./NodeDrawer";
import Legend from "./Legend";
import SearchBox from "./SearchBox";
import "./map.css";

function pinHtml(name: string, status: string, selected: boolean): string {
  const glyph = status === "planned" ? "◇" : status === "under_construction" ? "◈" : "◆";
  // 22px rotated-square diamond as SVG — dashed strokes stay crisp
  const stroke =
    status === "planned" ? "var(--bp-400)" : status === "under_construction" ? "var(--flow-warn)" : "var(--signal-500)";
  const dash = status === "built" ? "" : 'stroke-dasharray="4 3"';
  const fill =
    status === "built" ? "var(--steel-800)" : status === "planned" ? "rgba(86,168,255,.10)" : "rgba(138,100,35,.4)";
  return `
    <div class="pin-wrap">
      <svg class="pin-svg ${selected ? "selected" : ""}" width="30" height="30" viewBox="0 0 30 30">
        <rect x="8" y="8" width="14" height="14" transform="rotate(45 15 15)"
              fill="${fill}" stroke="${stroke}" stroke-width="2" ${dash} />
      </svg>
      <div class="pin-chip mono ${status}">${glyph} ${name.toUpperCase()}</div>
    </div>`;
}

export default function MapView() {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const layerRef = useRef<MapCanvasLayer | null>(null);
  const markersRef = useRef<Map<string, L.Marker>>(new Map());

  const plan = useStore((s) => s.plan);
  const world = useStore((s) => s.world);
  const derived = useStore((s) => s.derived);
  const overlays = useStore((s) => s.overlays);
  const selection = useStore((s) => s.selection);
  const placing = useStore((s) => s.placingFactory);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const setOverlay = useStore((s) => s.setOverlay);
  const setPlacing = useStore((s) => s.setPlacingFactory);
  const dispatch = useStore((s) => s.dispatch);

  const [hoveredNode, setHoveredNode] = useState<WorldNode | null>(null);
  const [zoomPct, setZoomPct] = useState(100);

  const nodeStates = useMemo(() => {
    const out: Record<string, NodeRenderState> = {};
    const claimsByNode: Record<string, number> = {};
    for (const c of Object.values(plan.nodeClaims)) {
      claimsByNode[c.node] = (claimsByNode[c.node] ?? 0) + 1;
    }
    for (const n of world.nodes) {
      const claims = claimsByNode[n.id] ?? 0;
      out[n.id] = {
        claims,
        claimed: claims > 0,
        conflict: derived.nodes[n.id]?.conflict ?? false,
      };
    }
    return out;
  }, [plan.nodeClaims, world.nodes, derived.nodes]);

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
      hoveredNode: null,
      selectedNode: null,
      showNodes: true,
    });
    layer.addTo(map);
    layerRef.current = layer;
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
    layerRef.current?.setData({
      world,
      nodeStates,
      hoveredNode: hoveredNode?.id ?? null,
      selectedNode: selection?.kind === "node" ? selection.id : null,
      showNodes: overlays.nodes,
    });
  }, [world, nodeStates, hoveredNode, selection, overlays.nodes]);

  // ---- pointer interactions (hover + click on canvas nodes, placement) ----
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    const onMove = (e: L.LeafletMouseEvent) => {
      const hit = layerRef.current?.hitTest(map.latLngToContainerPoint(e.latlng)) ?? null;
      setHoveredNode(hit);
      map.getContainer().style.cursor = placing ? "crosshair" : hit ? "pointer" : "";
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
      const hit = layerRef.current?.hitTest(map.latLngToContainerPoint(e.latlng));
      if (hit) setSelection({ kind: "node", id: hit.id });
      else setSelection(null);
    };
    map.on("mousemove", onMove);
    map.on("click", onClick);
    return () => {
      map.off("mousemove", onMove);
      map.off("click", onClick);
    };
  }, [placing, world.regions, dispatch, setPlacing, setSelection]);

  // ---- factory pin markers ----
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    const markers = markersRef.current;
    const seen = new Set<string>();
    for (const f of Object.values(plan.factories)) {
      seen.add(f.id);
      const selected = selection?.kind === "factory" && selection.id === f.id;
      const html = pinHtml(f.name, f.status, selected);
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
        marker.on("dragend", () => {
          const pos = fromLatLng(marker!.getLatLng());
          void useStore.getState().dispatch([{ type: "move_factory_pin", id: f.id, position: pos }]);
        });
        marker.addTo(map);
        markers.set(f.id, marker);
      } else {
        marker.setIcon(icon);
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
  }, [plan.factories, selection]);

  // ---- keys: N place, F frame, ESC deselect, 1/4 overlays, ⏎ dive ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
      const map = mapRef.current;
      if (e.key === "n" || e.key === "N") setPlacing(!placing);
      else if (e.key === "Escape") {
        if (placing) setPlacing(false);
        else setSelection(null);
      } else if (e.key === "1") setOverlay("flows", !overlays.flows);
      else if (e.key === "4") setOverlay("nodes", !overlays.nodes);
      else if (e.key === "f" || e.key === "F") {
        const pts = Object.values(plan.factories).map((f) => toLatLng(f.position));
        if (pts.length && map) map.fitBounds(L.latLngBounds(pts as L.LatLngExpression[]).pad(0.4));
      } else if (e.key === "Enter" && selection?.kind === "factory") {
        setView({ mode: "factory", factoryId: selection.id });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [placing, overlays, plan.factories, selection, setOverlay, setPlacing, setSelection, setView]);

  const panTo = useCallback((pos: { x: number; y: number }) => {
    mapRef.current?.panTo(toLatLng(pos));
  }, []);

  const zoomBy = (d: number) => mapRef.current?.setZoom((mapRef.current?.getZoom() ?? 2) + d);

  const selectedFactory = selection?.kind === "factory" ? plan.factories[selection.id] : null;
  const selectedNode = selection?.kind === "node" ? world.nodes.find((n) => n.id === selection.id) : null;

  return (
    <div className="map-root" data-testid="map-root">
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
            className={`btn btn-ghost overlay-chip ${overlays.nodes ? "active" : ""}`}
            onClick={() => setOverlay("nodes", !overlays.nodes)}
            title="Resource nodes (4)"
          >
            NODES <span className="key-hint">4</span>
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
    </div>
  );
}

function NodeTooltip({ node }: { node: WorldNode }) {
  const items = useStore((s) => s.gamedata.items);
  const name = items[node.item]?.displayName ?? node.item;
  return (
    <div className="node-tooltip chip">
      {name.toUpperCase()} · {node.purity.toUpperCase()}
    </div>
  );
}
