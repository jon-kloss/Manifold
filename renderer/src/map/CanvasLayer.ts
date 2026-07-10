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

export interface CanvasLayerData {
  world: World;
  nodeStates: Record<string, NodeRenderState>;
  hoveredNode: string | null;
  selectedNode: string | null;
  showNodes: boolean;
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
    return `${code} ${purity}`;
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
    if (this.data.showNodes) this.drawNodes(ctx, map);
  };

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
