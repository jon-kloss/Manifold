// Custom canvas overlay (SDD §2: canvas from day one — no thousand DOM markers).
// Draws the survey grid, biome ghost labels, and resource nodes with the
// purity/claim/conflict grammar from mock 2a. Factory pins stay DOM (rich drag
// interactions, few of them).

import L from "leaflet";
import type { World, WorldNode } from "../state/types";
import { toLatLng } from "./maputil";

export interface NodeRenderState {
  claims: number;
  conflict: boolean;
  claimed: boolean;
}

export interface RouteRender {
  id: string;
  path: { x: number; y: number }[];
  planned: boolean;
  saturation: number;
  flow: number;
  capacity: number;
  tier: number;
  itemName: string;
  selected: boolean;
}

export interface CanvasLayerData {
  world: World;
  nodeStates: Record<string, NodeRenderState>;
  hoveredNode: string | null;
  selectedNode: string | null;
  showNodes: boolean;
  routes: RouteRender[];
  showRoutes: boolean;
  /** power lines (pairs of factory positions) + grid chips at centroids */
  powerLines: { from: { x: number; y: number }; to: { x: number; y: number }; selected: boolean; id: string }[];
  circuitChips: { x: number; y: number; text: string; level: "ok" | "warn" | "crit" }[];
  /** priority switches (A2.3): square pins on power lines + P/SHEDS chip */
  switches: { id: string; x: number; y: number; priority: number; chip: string; selected: boolean }[];
  showPower: boolean;
  /** right-drag route ghost (blueprint-dashed until confirmed) */
  ghost: { from: { x: number; y: number }; to: { x: number; y: number } } | null;
  /** proposal review (mock 3a): world dims, ghosts render in status grammar */
  review: {
    pins: { x: number; y: number; name: string }[];
    claimRings: { x: number; y: number }[];
    modifyRings: { x: number; y: number }[];
    lines: { from: { x: number; y: number }; to: { x: number; y: number }; power: boolean }[];
  } | null;
}

const css = (name: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim();

export class MapCanvasLayer extends L.Layer {
  private canvas: HTMLCanvasElement | null = null;
  private data: CanvasLayerData;
  private mapRef: L.Map | null = null;

  constructor(data: CanvasLayerData) {
    super();
    this.data = data;
  }

  setData(data: CanvasLayerData) {
    this.data = data;
    this.redraw();
  }

  onAdd(map: L.Map): this {
    this.mapRef = map;
    const canvas = document.createElement("canvas");
    canvas.className = "map-canvas-layer";
    canvas.style.position = "absolute";
    canvas.style.inset = "0";
    canvas.style.pointerEvents = "none";
    canvas.style.zIndex = "200";
    map.getContainer().appendChild(canvas);
    this.canvas = canvas;
    map.on("move zoom viewreset resize", this.redraw, this);
    this.redraw();
    return this;
  }

  onRemove(map: L.Map): this {
    map.off("move zoom viewreset resize", this.redraw, this);
    this.canvas?.remove();
    this.canvas = null;
    this.mapRef = null;
    return this;
  }

  /** Hit-test nodes in container-pixel space (14px circles, 10px slop). */
  hitTest(point: L.Point): WorldNode | null {
    const map = this.mapRef;
    if (!map || !this.data.showNodes) return null;
    for (const node of this.data.world.nodes) {
      const p = map.latLngToContainerPoint(toLatLng(node));
      const dx = p.x - point.x;
      const dy = p.y - point.y;
      if (dx * dx + dy * dy <= 12 * 12) return node;
    }
    return null;
  }

  /** Short label for a node: FE PURE, CU NORM, LIME IMP… */
  private nodeLabel(node: WorldNode): string {
    const code =
      { Desc_OreIron_C: "FE", Desc_OreCopper_C: "CU", Desc_Stone_C: "LIME", Desc_Coal_C: "COAL" }[node.item] ??
      node.item.replace("Desc_", "").replace("_C", "").slice(0, 4).toUpperCase();
    const purity = node.purity === "normal" ? "NORM" : node.purity.toUpperCase();
    return node.zone === "cave" ? `${code} ${purity} ▾CAVE` : `${code} ${purity}`;
  }

  redraw = () => {
    const map = this.mapRef;
    const canvas = this.canvas;
    if (!map || !canvas) return;
    const size = map.getSize();
    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== size.x * dpr || canvas.height !== size.y * dpr) {
      canvas.width = size.x * dpr;
      canvas.height = size.y * dpr;
      canvas.style.width = `${size.x}px`;
      canvas.style.height = `${size.y}px`;
    }
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.x, size.y);

    this.drawRegionTints(ctx, map);
    this.drawGrid(ctx, map, size);
    this.drawRegionLabels(ctx, map);
    if (this.data.showPower) this.drawPower(ctx, map);
    if (this.data.showRoutes) this.drawRoutes(ctx, map);
    if (this.data.showNodes) this.drawNodes(ctx, map);
    this.drawGhost(ctx, map);
    if (this.data.review) this.drawReview(ctx, map, size);
  };

  /** Review mode: dim the world to 42%, then draw the proposal's ghosts at
   *  full strength — new sites as blueprint pins, claims/modifies as rings,
   *  routes blueprint-dashed. DOM pins dim via CSS (.reviewing). */
  private drawReview(ctx: CanvasRenderingContext2D, map: L.Map, size: L.Point) {
    const review = this.data.review!;
    ctx.fillStyle = "rgba(10, 12, 14, 0.58)";
    ctx.fillRect(0, 0, size.x, size.y);

    for (const l of review.lines) {
      const a = map.latLngToContainerPoint(toLatLng(l.from));
      const b = map.latLngToContainerPoint(toLatLng(l.to));
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = css("--bp-400");
      ctx.lineWidth = 2;
      ctx.setLineDash(l.power ? [3, 5] : [8, 6]);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    for (const r of review.claimRings) {
      const p = map.latLngToContainerPoint(toLatLng(r));
      ctx.beginPath();
      ctx.arc(p.x, p.y, 13, 0, Math.PI * 2);
      ctx.strokeStyle = css("--bp-400");
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 3]);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    for (const r of review.modifyRings) {
      const p = map.latLngToContainerPoint(toLatLng(r));
      ctx.beginPath();
      ctx.arc(p.x, p.y, 20, 0, Math.PI * 2);
      ctx.strokeStyle = css("--flow-warn");
      ctx.lineWidth = 1.5;
      ctx.setLineDash([5, 4]);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.font = `700 11px ${css("--font-mono")}`;
      ctx.fillStyle = css("--flow-warn");
      ctx.fillText("Δ", p.x + 24, p.y + 4);
    }
    for (const pin of review.pins) {
      const p = map.latLngToContainerPoint(toLatLng(pin));
      // blueprint ghost diamond
      ctx.save();
      ctx.translate(p.x, p.y);
      ctx.rotate(Math.PI / 4);
      ctx.strokeStyle = css("--bp-400");
      ctx.fillStyle = "rgba(86, 168, 255, .12)";
      ctx.lineWidth = 2;
      ctx.setLineDash([4, 3]);
      ctx.strokeRect(-8, -8, 16, 16);
      ctx.fillRect(-8, -8, 16, 16);
      ctx.restore();
      ctx.setLineDash([]);
      // inverted blue chip: + NAME — NEW
      const text = `+ ${pin.name.toUpperCase()} — NEW`;
      ctx.font = `700 10px ${css("--font-mono")}`;
      const w = ctx.measureText(text).width + 12;
      ctx.fillStyle = css("--bp-400");
      ctx.fillRect(p.x - w / 2, p.y + 14, w, 16);
      ctx.fillStyle = css("--steel-950");
      ctx.textAlign = "center";
      ctx.fillText(text, p.x, p.y + 25);
      ctx.textAlign = "left";
    }
  }

  /** Flow/route encoding per mock 1e. Planned routes are always
   *  blueprint-dashed; saturation rides the label chip (color + italic). */
  private drawRoutes(ctx: CanvasRenderingContext2D, map: L.Map) {
    for (const r of this.data.routes) {
      const pts = r.path.map((p) => map.latLngToContainerPoint(toLatLng(p)));
      if (pts.length < 2) continue;
      const level = r.saturation >= 0.95 ? "crit" : r.saturation >= 0.7 ? "warn" : "ok";
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (const p of pts.slice(1)) ctx.lineTo(p.x, p.y);
      if (r.planned) {
        ctx.strokeStyle = r.selected ? css("--signal-500") : css("--bp-400");
        ctx.lineWidth = 2;
        ctx.setLineDash([8, 6]);
      } else {
        ctx.strokeStyle = r.selected
          ? css("--signal-500")
          : css(level === "crit" ? "--flow-crit" : level === "warn" ? "--flow-warn" : "--flow-ok");
        ctx.lineWidth = level === "crit" ? 6 : level === "warn" ? 4 : 2;
        ctx.setLineDash(level === "ok" ? [] : level === "warn" ? [10, 5] : [6, 4]);
      }
      ctx.stroke();
      ctx.setLineDash([]);

      // label chip at the midpoint — the always-present data channel
      const mid = pts[Math.floor((pts.length - 1) / 2)];
      const mid2 = pts[Math.min(pts.length - 1, Math.floor((pts.length - 1) / 2) + 1)];
      const cx = (mid.x + mid2.x) / 2;
      const cy = (mid.y + mid2.y) / 2;
      const text = `${r.itemName} · ${Math.round(r.flow * 100) / 100}/${r.capacity} · ${Math.round(
        r.saturation * 100,
      )}%  MK.${r.tier}`;
      ctx.font = `italic 500 9px ${css("--font-mono")}`;
      const w = ctx.measureText(text).width + 10;
      ctx.fillStyle = level === "crit" ? css("--flow-crit") : css("--steel-800");
      ctx.strokeStyle = r.selected ? css("--signal-500") : css("--steel-600");
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.rect(cx - w / 2, cy - 8, w, 16);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle =
        level === "crit" ? css("--on-signal") : level === "warn" ? css("--flow-warn") : css("--bp-400");
      ctx.textAlign = "center";
      ctx.fillText(text, cx, cy + 3);
      ctx.textAlign = "left";
    }
  }

  /** Power lines: single 2px line; the chip carries the circuit margin, not
   *  link load — power is a bus, not a belt (A2.1). */
  private drawPower(ctx: CanvasRenderingContext2D, map: L.Map) {
    for (const l of this.data.powerLines) {
      const a = map.latLngToContainerPoint(toLatLng(l.from));
      const b = map.latLngToContainerPoint(toLatLng(l.to));
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = l.selected ? css("--signal-500") : css("--bp-400");
      ctx.lineWidth = 2;
      ctx.setLineDash([8, 6]); // planned grammar
      ctx.stroke();
      ctx.setLineDash([]);
    }
    for (const sw of this.data.switches) {
      const p = map.latLngToContainerPoint(toLatLng(sw));
      // 18px square pin — square = infrastructure (A2.3 grammar)
      ctx.fillStyle = css("--steel-800");
      ctx.strokeStyle = sw.selected ? css("--signal-500") : css("--bp-400");
      ctx.lineWidth = 2;
      ctx.setLineDash([4, 3]); // planned
      ctx.fillRect(p.x - 9, p.y - 9, 18, 18);
      ctx.strokeRect(p.x - 9, p.y - 9, 18, 18);
      ctx.setLineDash([]);
      ctx.font = `700 9px ${css("--font-mono")}`;
      ctx.fillStyle = css("--ink-100");
      ctx.textAlign = "center";
      ctx.fillText(`P${sw.priority}`, p.x, p.y + 3);
      ctx.textAlign = "left";
      if (sw.chip) {
        ctx.font = `500 9px ${css("--font-mono")}`;
        const w = ctx.measureText(sw.chip).width + 10;
        ctx.fillStyle = css("--steel-800");
        ctx.strokeStyle = sw.selected ? css("--signal-500") : css("--steel-600");
        ctx.lineWidth = 1;
        ctx.fillRect(p.x - w / 2, p.y + 12, w, 15);
        ctx.strokeRect(p.x - w / 2, p.y + 12, w, 15);
        ctx.fillStyle = css("--ink-300");
        ctx.textAlign = "center";
        ctx.fillText(sw.chip, p.x, p.y + 23);
        ctx.textAlign = "left";
      }
    }
    for (const c of this.data.circuitChips) {
      const p = map.latLngToContainerPoint(toLatLng(c));
      ctx.font = `700 10px ${css("--font-mono")}`;
      const w = ctx.measureText(c.text).width + 12;
      ctx.fillStyle = css("--steel-800");
      ctx.strokeStyle = css(c.level === "crit" ? "--flow-crit" : c.level === "warn" ? "--flow-warn-dark" : "--steel-600");
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.rect(p.x - w / 2, p.y - 10, w, 20);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = css(c.level === "crit" ? "--flow-crit" : c.level === "warn" ? "--flow-warn" : "--flow-ok");
      ctx.textAlign = "center";
      ctx.fillText(c.text, p.x, p.y + 4);
      ctx.textAlign = "left";
    }
  }

  /** Switch square under the pointer (checked before line hits). */
  hitTestSwitch(point: L.Point): string | null {
    const map = this.mapRef;
    if (!map || !this.data.showPower) return null;
    for (const sw of this.data.switches) {
      const p = map.latLngToContainerPoint(toLatLng(sw));
      if (Math.abs(point.x - p.x) <= 11 && Math.abs(point.y - p.y) <= 11) return sw.id;
    }
    return null;
  }

  /** Nearest power line within 8px. */
  hitTestPower(point: L.Point): string | null {
    const map = this.mapRef;
    if (!map || !this.data.showPower) return null;
    for (const l of this.data.powerLines) {
      const a = map.latLngToContainerPoint(toLatLng(l.from));
      const b = map.latLngToContainerPoint(toLatLng(l.to));
      if (distToSegment(point, a, b) < 8) return l.id;
    }
    return null;
  }

  private drawGhost(ctx: CanvasRenderingContext2D, map: L.Map) {
    const g = this.data.ghost;
    if (!g) return;
    const a = map.latLngToContainerPoint(toLatLng(g.from));
    const b = map.latLngToContainerPoint(toLatLng(g.to));
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.strokeStyle = css("--bp-400");
    ctx.lineWidth = 2;
    ctx.setLineDash([8, 6]);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  /** Nearest route within 8px of a container point. */
  hitTestRoute(point: L.Point): string | null {
    const map = this.mapRef;
    if (!map || !this.data.showRoutes) return null;
    for (const r of this.data.routes) {
      const pts = r.path.map((p) => map.latLngToContainerPoint(toLatLng(p)));
      for (let i = 0; i < pts.length - 1; i++) {
        if (distToSegment(point, pts[i], pts[i + 1]) < 8) return r.id;
      }
    }
    return null;
  }

  /** Faint elliptical tint per region — placeholder world-imagery treatment. */
  private drawRegionTints(ctx: CanvasRenderingContext2D, map: L.Map) {
    for (const region of this.data.world.regions) {
      const c = map.latLngToContainerPoint(toLatLng({ x: region.labelX, y: region.labelY }));
      const r = map.latLngToContainerPoint(toLatLng({ x: region.labelX + 1200, y: region.labelY })).x - c.x;
      const g = ctx.createRadialGradient(c.x, c.y, 0, c.x, c.y, Math.max(60, r));
      g.addColorStop(0, "rgba(236,238,240,0.020)");
      g.addColorStop(1, "rgba(236,238,240,0)");
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.ellipse(c.x, c.y, Math.max(60, r), Math.max(45, r * 0.72), 0, 0, Math.PI * 2);
      ctx.fill();
    }
  }

  /** Survey grid: world-anchored lines whose screen spacing stays near 160px. */
  private drawGrid(ctx: CanvasRenderingContext2D, map: L.Map, size: L.Point) {
    const gridColor = css("--map-grid");
    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 1;
    const nw = map.containerPointToLatLng(L.point(0, 0));
    const se = map.containerPointToLatLng(L.point(size.x, size.y));
    // world meters per screen px
    const metersPerPx = ((se.lng - nw.lng) * 50) / size.x;
    const targetMeters = 160 * metersPerPx;
    const step = Math.pow(2, Math.round(Math.log2(Math.max(1, targetMeters))));
    const startX = Math.floor((nw.lng * 50) / step) * step;
    const endX = se.lng * 50;
    for (let wx = startX; wx <= endX; wx += step) {
      const p = map.latLngToContainerPoint([0, wx / 50]);
      ctx.beginPath();
      ctx.moveTo(Math.round(p.x) + 0.5, 0);
      ctx.lineTo(Math.round(p.x) + 0.5, size.y);
      ctx.stroke();
    }
    const startY = Math.floor((-nw.lat * 50) / step) * step;
    const endY = -se.lat * 50;
    for (let wy = startY; wy <= endY + step; wy += step) {
      const p = map.latLngToContainerPoint([-wy / 50, 0]);
      ctx.beginPath();
      ctx.moveTo(0, Math.round(p.y) + 0.5);
      ctx.lineTo(size.x, Math.round(p.y) + 0.5);
      ctx.stroke();
    }
  }

  private drawRegionLabels(ctx: CanvasRenderingContext2D, map: L.Map) {
    ctx.font = `500 10px ${css("--font-mono")}`;
    ctx.fillStyle = css("--ink-ghost");
    ctx.letterSpacing = "2px";
    for (const region of this.data.world.regions) {
      const p = map.latLngToContainerPoint(toLatLng({ x: region.labelX, y: region.labelY }));
      ctx.fillText(region.name, p.x, p.y);
    }
    ctx.letterSpacing = "0px";
  }

  private drawNodes(ctx: CanvasRenderingContext2D, map: L.Map) {
    const inkMuted = css("--ink-500");
    const signal = css("--signal-500");
    const crit = css("--flow-crit");
    const canvasBg = css("--map-canvas");

    for (const node of this.data.world.nodes) {
      const state = this.data.nodeStates[node.id] ?? { claims: 0, conflict: false, claimed: false };
      const p = map.latLngToContainerPoint(toLatLng(node));
      const r = 7;
      const hovered = this.data.hoveredNode === node.id;
      const selected = this.data.selectedNode === node.id;

      // halo so nodes read over the grid
      ctx.beginPath();
      ctx.arc(p.x, p.y, r + 3, 0, Math.PI * 2);
      ctx.fillStyle = canvasBg;
      ctx.fill();

      // purity ring: pure solid / normal dashed / impure dotted
      ctx.beginPath();
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
      ctx.lineWidth = hovered || selected ? 2 : 1.5;
      ctx.strokeStyle = state.conflict ? crit : hovered || selected ? css("--ink-100") : inkMuted;
      if (node.purity === "normal") ctx.setLineDash([4, 3]);
      else if (node.purity === "impure") ctx.setLineDash([1.5, 2.5]);
      else ctx.setLineDash([]);
      ctx.stroke();
      ctx.setLineDash([]);

      // cave nodes: an under-arc below the ring (underground), plus the
      // surface entrance — a small square (infrastructure, A2.3) linked by a
      // dotted line while hovered/selected so routing via it reads naturally
      if (node.zone === "cave") {
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 3.5, Math.PI * 0.15, Math.PI * 0.85);
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = hovered || selected ? css("--ink-100") : inkMuted;
        ctx.stroke();
        if (node.entrance && (hovered || selected)) {
          const e = map.latLngToContainerPoint(toLatLng(node.entrance));
          ctx.beginPath();
          ctx.moveTo(p.x, p.y);
          ctx.lineTo(e.x, e.y);
          ctx.setLineDash([2, 4]);
          ctx.lineWidth = 1;
          ctx.strokeStyle = inkMuted;
          ctx.stroke();
          ctx.setLineDash([]);
          ctx.strokeRect(e.x - 3.5, e.y - 3.5, 7, 7);
          ctx.font = `500 9px ${css("--font-mono")}`;
          ctx.fillStyle = inkMuted;
          ctx.fillText("ENTRANCE", e.x + 7, e.y + 3);
        }
      }

      // claimed = orange center dot; free = hollow
      if (state.claimed) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, 3, 0, Math.PI * 2);
        ctx.fillStyle = state.conflict ? crit : signal;
        ctx.fill();
      }

      // mono ghost label under every node (mock 2a: FE PURE #08)
      ctx.font = `500 9px ${css("--font-mono")}`;
      ctx.textAlign = "center";
      ctx.fillStyle = state.conflict ? crit : hovered || selected ? inkMuted : css("--ink-ghost");
      ctx.fillText(this.nodeLabel(node), p.x, p.y + r + 12);
      ctx.textAlign = "left";

      if (state.conflict) {
        ctx.font = `700 9px ${css("--font-mono")}`;
        ctx.fillStyle = crit;
        ctx.fillText(`⚠×${state.claims}`, p.x + r + 4, p.y + 3);
      }

      if (selected) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 5, 0, Math.PI * 2);
        ctx.strokeStyle = signal;
        ctx.lineWidth = 1.5;
        ctx.stroke();
      }
    }
  }
}

function distToSegment(p: L.Point, a: L.Point, b: L.Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
  const px = a.x + t * dx;
  const py = a.y + t * dy;
  return Math.hypot(p.x - px, p.y - py);
}
