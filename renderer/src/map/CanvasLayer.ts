// Custom canvas overlay (SDD §2: canvas from day one — no thousand DOM markers).
// Draws the survey grid, biome ghost labels, and resource nodes with the
// purity/claim/conflict grammar from mock 2a. Factory pins stay DOM (rich drag
// interactions, few of them).

import L from "leaflet";
import type { World, WorldNode } from "../state/types";
import { flowBand } from "../lib/format";
import { toLatLng } from "./maputil";

export interface NodeRenderState {
  claims: number;
  conflict: boolean;
  claimed: boolean;
  /** W2b-C: the node sits at a plan-corrected position (save disagreed with the
   *  catalog) — draw a small drift marker. */
  drift?: boolean;
}

export interface RouteRender {
  id: string;
  /** parallel-route fan: lane index / total lanes between this factory pair */
  lane: number;
  lanes: number;
  path: { x: number; y: number }[];
  planned: boolean;
  saturation: number;
  flow: number;
  capacity: number;
  /** honest red: downstream registers a deficit through this route while it
   *  runs at full capacity (MapView derives it from Derived.deficits) */
  bottleneck: boolean;
  /** transport kind — drives the on-line glyph notation (ties, squares, dots) */
  kind: "belt" | "rail" | "truck" | "drone";
  /** label chip suffix: MK.n for belts, RAIL/TRUCK/DRONE for transports */
  tag: string;
  itemName: string;
  selected: boolean;
}

export interface CanvasLayerData {
  world: World;
  nodeStates: Record<string, NodeRenderState>;
  /** node→claiming-factory tethers (the assignment made visible) */
  claimLinks: {
    node: { x: number; y: number };
    factory: { x: number; y: number };
    factoryName: string;
    planned: boolean;
    conflict: boolean;
    highlight: boolean;
  }[];
  /** old ◆ → new ◇ refactor tethers (W2a): the "this replaces that" link */
  replacesLinks: {
    old: { x: number; y: number };
    new: { x: number; y: number };
    highlight: boolean;
  }[];
  hoveredNode: string | null;
  selectedNode: string | null;
  showNodes: boolean;
  /** Live search filter: when active, only nodes in `visible` are drawn and
   *  hit-tested — the rest are toggled OFF so typing narrows the map. */
  nodeFilter: { active: boolean; visible: Set<string> } | null;
  /** real-world terrain underlay (drawn at the bottom of this canvas) */
  showTerrain: boolean;
  routes: RouteRender[];
  showRoutes: boolean;
  /** power lines (pairs of factory positions) + grid chips at centroids */
  powerLines: {
    from: { x: number; y: number };
    to: { x: number; y: number };
    selected: boolean;
    id: string;
    lane: number;
    lanes: number;
  }[];
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

// Real-world terrain calibration (community render; provenance in NOTICE).
// Standard map bounds: X -324,698.83..+425,301.83 cm, Y ±375,000 cm → meters.
// Image row 0 = north = -Y (toLatLng puts -y at high lat).
const TERRAIN_BOUNDS = { minX: -3246.98832031, maxX: 4253.01832031, minY: -3750, maxY: 3750 };
const TERRAIN_FILTER = "saturate(0.5) brightness(0.55) contrast(1.05)";
const TERRAIN_URL = "/map/world.webp";

// MOTION = FLOW (gate: flow > 0); speed = utilization (route animation).
// The moving dash phase lives on its
// own lightweight canvas above the data canvas — the full redraw (terrain
// blit + 459 nodes + grid + chips) is far too hot to run per frame on an
// 80-factory world, but re-stroking only the flowing polylines is ~1 ms.
// The RAF loop is throttled to ≤24 fps and runs ONLY while at least one
// flowing route is visible, the tab is visible, and reduced-motion is off.
const ANIM_FPS = 24;
/** one dash period of the moving highlight, px */
const ANIM_PERIOD = 18;
/** phase speed, px/s: slow trickle at 0 utilization → fast when saturated.
 *  Speed encodes utilization WITHIN a surface; the absolute px/s curve here
 *  is tuned for map world-scale legibility (the graph's flowSpeed in
 *  lib/format.ts is card-scale tuned) — deliberately not shared. */
const animSpeed = (saturation: number) => 14 + 46 * Math.max(0, Math.min(1, saturation));

export class MapCanvasLayer extends L.Layer {
  private canvas: HTMLCanvasElement | null = null;
  /** Label-chip rects placed this redraw — later overlapping chips are culled
   *  (their lines/pins still draw). Selected chips are placed first. */
  private chipRects: { x: number; y: number; w: number; h: number }[] = [];
  private data: CanvasLayerData;
  private mapRef: L.Map | null = null;
  /** terrain pre-rendered once with the muted design filter baked in */
  private terrainCanvas: HTMLCanvasElement | null = null;
  private terrainLoading = false;
  /** Projected node container-points cached during drawNodes so hitTestNode
   *  reuses them instead of re-projecting all 459 nodes per mousemove.
   *  Rebuilt each redraw (which already fires on move/zoom/viewreset/resize). */
  private nodeScreen: { node: WorldNode; x: number; y: number }[] = [];
  /** flow-animation overlay (see ANIM_FPS note above) */
  private animCanvas: HTMLCanvasElement | null = null;
  private animRaf: number | null = null;
  private animLastT = 0;
  private reduceMotion: MediaQueryList | null = null;

  constructor(data: CanvasLayerData) {
    super();
    this.data = data;
  }

  setData(data: CanvasLayerData) {
    this.data = data;
    this.redraw();
    this.syncAnimLoop();
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
    const anim = document.createElement("canvas");
    anim.className = "map-anim-layer";
    anim.style.position = "absolute";
    anim.style.inset = "0";
    anim.style.pointerEvents = "none";
    anim.style.zIndex = "201";
    map.getContainer().appendChild(anim);
    this.animCanvas = anim;
    this.reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
    this.reduceMotion.addEventListener("change", this.syncAnimLoop);
    document.addEventListener("visibilitychange", this.syncAnimLoop);
    void this.loadTerrain();
    map.on("move zoom viewreset resize", this.redraw, this);
    // Follow Leaflet's zoom animation instead of snapping at the end: the
    // container-fixed canvas gets a matching scale-about-pivot transform so the
    // nodes/routes glide with the pins and tiles, then redraw crisp at zoomend.
    map.on("zoomanim", this.animateZoom, this);
    this.redraw();
    this.syncAnimLoop();
    return this;
  }

  onRemove(map: L.Map): this {
    map.off("move zoom viewreset resize", this.redraw, this);
    map.off("zoomanim", this.animateZoom, this);
    if (this.animRaf != null) cancelAnimationFrame(this.animRaf);
    this.animRaf = null;
    this.reduceMotion?.removeEventListener("change", this.syncAnimLoop);
    document.removeEventListener("visibilitychange", this.syncAnimLoop);
    this.reduceMotion = null;
    this.animCanvas?.remove();
    this.animCanvas = null;
    this.canvas?.remove();
    this.canvas = null;
    this.mapRef = null;
    return this;
  }

  /** Routes that animate: derived flow > 0 while the flow layer is shown.
   *  Idle and planned-but-unfed routes stay static (MOTION = FLOW; speed =
   *  utilization). */
  private flowingRoutes(): RouteRender[] {
    // review mode dims the world under the proposal ghosts — bright moving
    // dashes above the dim scrim would fight the review focus, so pause
    if (!this.data.showRoutes || this.data.review) return [];
    return this.data.routes.filter((r) => r.flow > 0);
  }

  /** Start/stop the RAF loop so the animation never burns background CPU:
   *  it runs only with ≥1 flowing route, a visible document, and no
   *  reduced-motion preference. */
  private syncAnimLoop = () => {
    const want =
      this.animCanvas != null &&
      this.flowingRoutes().length > 0 &&
      document.visibilityState === "visible" &&
      !this.reduceMotion?.matches;
    if (want && this.animRaf == null) {
      this.animRaf = requestAnimationFrame(this.animFrame);
    } else if (!want && this.animRaf != null) {
      cancelAnimationFrame(this.animRaf);
      this.animRaf = null;
      const ctx = this.animCanvas?.getContext("2d");
      if (ctx && this.animCanvas) ctx.clearRect(0, 0, this.animCanvas.width, this.animCanvas.height);
    }
  };

  private animFrame = (t: number) => {
    this.animRaf = requestAnimationFrame(this.animFrame);
    if (t - this.animLastT < 1000 / ANIM_FPS) return;
    this.animLastT = t;
    this.drawAnim(t);
  };

  /** Stroke ONLY the flowing routes' moving-dash highlight. Neutral ink over
   *  the base line: the status color underneath stays untouched (color is
   *  status-only; motion is the orthogonal throughput channel). Phase is
   *  time-based (px/s), so a dropped frame never changes perceived speed;
   *  path order is from → to, and a negative dash offset moves the dashes
   *  in that direction. */
  private drawAnim(t: number) {
    const map = this.mapRef;
    const canvas = this.animCanvas;
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
    ctx.lineCap = "round";
    const ink = css("--ink-100");
    const secs = t / 1000;
    for (const r of this.flowingRoutes()) {
      const pts = this.lanePts(r.path.map((p) => map.latLngToContainerPoint(toLatLng(p))), r.lane, r.lanes);
      if (pts.length < 2) continue;
      ctx.beginPath();
      ctx.moveTo(pts[0].x, pts[0].y);
      for (const p of pts.slice(1)) ctx.lineTo(p.x, p.y);
      ctx.setLineDash([6, ANIM_PERIOD - 6]);
      ctx.lineDashOffset = -((secs * animSpeed(r.saturation)) % ANIM_PERIOD);
      // Dark casing first, then the bright beads on top: the casing gives the
      // light dash a contrasting edge so it stays obvious over the light amber
      // (under-used) band, where a translucent white dash used to wash out.
      ctx.strokeStyle = "rgba(6, 10, 14, 0.85)";
      ctx.globalAlpha = 1;
      ctx.lineWidth = 5;
      ctx.stroke();
      ctx.strokeStyle = ink;
      ctx.globalAlpha = 0.95;
      ctx.lineWidth = 3;
      ctx.stroke();
    }
    ctx.setLineDash([]);
    ctx.lineDashOffset = 0;
    ctx.globalAlpha = 1;
  }

  /** Hit-test nodes in container-pixel space: NEAREST node within 12px, with
   *  its distance — callers use the distance to arbitrate against route hits
   *  (459 real nodes make "first within slop" steal clicks aimed at lines). */
  hitTestNode(point: L.Point): { node: WorldNode; d: number } | null {
    const map = this.mapRef;
    if (!map || !this.data.showNodes) return null;
    let best: { node: WorldNode; d: number } | null = null;
    // Fast path: the points cached by the last drawNodes. Falls back to live
    // projection before the first paint (empty cache).
    const cache = this.nodeScreen;
    if (cache.length) {
      for (const c of cache) {
        const dx = c.x - point.x;
        const dy = c.y - point.y;
        const d = Math.sqrt(dx * dx + dy * dy);
        if (d <= 12 && (!best || d < best.d)) best = { node: c.node, d };
      }
      return best;
    }
    const filter = this.data.nodeFilter;
    for (const node of this.data.world.nodes) {
      if (filter?.active && !filter.visible.has(node.id)) continue;
      const p = map.latLngToContainerPoint(toLatLng(node));
      const dx = p.x - point.x;
      const dy = p.y - point.y;
      const d = Math.sqrt(dx * dx + dy * dy);
      if (d <= 12 && (!best || d < best.d)) best = { node, d };
    }
    return best;
  }

  hitTest(point: L.Point): WorldNode | null {
    return this.hitTestNode(point)?.node ?? null;
  }

  /** Node label: the RESOURCE NAME players know (IRON PURE, COPPER NORM…), not
   *  the element symbol — FE/CU read as chemistry, not Satisfactory resources. */
  private nodeLabel(node: WorldNode): string {
    const name =
      {
        Desc_OreIron_C: "IRON",
        Desc_OreCopper_C: "COPPER",
        Desc_Stone_C: "LIMESTONE",
        Desc_Coal_C: "COAL",
        Desc_OreGold_C: "CATERIUM",
        Desc_RawQuartz_C: "QUARTZ",
        Desc_Sulfur_C: "SULFUR",
        Desc_LiquidOil_C: "OIL",
        Desc_OreBauxite_C: "BAUXITE",
        Desc_OreUranium_C: "URANIUM",
        Desc_SAM_C: "SAM",
      }[node.item] ??
      // save-only nodes carry item:"" — degrade to a readable NODE, not "".
      (node.item || "NODE").replace("Desc_", "").replace("_C", "").replace(/_/g, " ").toUpperCase();
    const purity = node.purity === "normal" ? "NORM" : node.purity.toUpperCase();
    return node.zone === "cave" ? `${name} ${purity} ▾CAVE` : `${name} ${purity}`;
  }

  /** Match Leaflet's zoom animation on the container-fixed overlays. Scaling
   *  about `pivot` (chosen so the animation's target centre lands at the
   *  container centre) reproduces Leaflet's end state exactly, so the crisp
   *  redraw at zoomend replaces the transform with no visible snap. A CSS
   *  transition matching Leaflet's (0.25s) makes the two glide in lockstep. */
  private animateZoom = (e: L.ZoomAnimEvent) => {
    const map = this.mapRef;
    if (!map) return;
    const scale = map.getZoomScale(e.zoom);
    if (scale === 1) return;
    const size = map.getSize();
    const ce = map.latLngToContainerPoint(e.center);
    const px = (size.x / 2 - scale * ce.x) / (1 - scale);
    const py = (size.y / 2 - scale * ce.y) / (1 - scale);
    for (const c of [this.canvas, this.animCanvas]) {
      if (!c) continue;
      c.style.transition = "transform 0.25s cubic-bezier(0,0,0.25,1)";
      c.style.transformOrigin = `${px}px ${py}px`;
      c.style.transform = `scale(${scale})`;
    }
  };

  redraw = () => {
    const map = this.mapRef;
    const canvas = this.canvas;
    if (!map || !canvas) return;
    // Drop any zoom-follow transform instantly (no reverse-glide) now that we
    // draw at the final zoom.
    for (const c of [canvas, this.animCanvas]) {
      if (c && c.style.transform) {
        c.style.transition = "none";
        c.style.transform = "";
      }
    }
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
    this.chipRects = [];

    if (this.data.showTerrain) this.drawTerrain(ctx, map, size);
    this.drawRegionTints(ctx, map);
    this.drawGrid(ctx, map, size);
    this.drawRegionLabels(ctx, map);
    if (this.data.showPower) this.drawPower(ctx, map);
    if (this.data.showRoutes) this.drawRoutes(ctx, map);
    if (this.data.showNodes) {
      this.drawClaimLinks(ctx, map);
      this.drawNodes(ctx, map);
    }
    this.drawReplacesLinks(ctx, map);
    this.drawGhost(ctx, map);
    if (this.data.review) this.drawReview(ctx, map, size);
    // keep the animated dashes in lockstep with the base lines during
    // pan/zoom — an extra overlay stroke here is cheap, the lag isn't
    if (this.animRaf != null) this.drawAnim(performance.now());
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

  /** Flow/route encoding (efficiency grammar). Planned routes are always
   *  blueprint-dashed; utilization rides the label chip (color + italic). */
  /** Walk a polyline and invoke fn at every `spacing` px (phase-offset by
   *  spacing/2 so glyphs stay clear of the endpoints) with the local angle. */
  private alongLine(
    pts: L.Point[],
    spacing: number,
    fn: (x: number, y: number, angle: number) => void,
    phase = 0,
  ) {
    let carry = spacing / 2 + phase;
    for (let i = 1; i < pts.length; i++) {
      const dx = pts[i].x - pts[i - 1].x;
      const dy = pts[i].y - pts[i - 1].y;
      const len = Math.hypot(dx, dy);
      if (len < 1e-3) continue;
      const angle = Math.atan2(dy, dx);
      let t = carry;
      while (t <= len) {
        fn(pts[i - 1].x + (dx * t) / len, pts[i - 1].y + (dy * t) / len, angle);
        t += spacing;
      }
      carry = t - len;
    }
  }

  private drawRoutes(ctx: CanvasRenderingContext2D, map: L.Map) {
    // pass 1: every line, with map-notation kind glyphs — the line itself
    // states direction and transport kind even when its label chip is culled
    for (const r of this.data.routes) {
      const pts = this.lanePts(r.path.map((p) => map.latLngToContainerPoint(toLatLng(p))), r.lane, r.lanes);
      if (pts.length < 2) continue;
      // efficiency grammar: green = good (incl. FULL meeting demand), amber
      // dashed = under-used (≤50%), heavy red = bottleneck (deficit through a
      // full route) — same authority as the graph edges (lib/format).
      const band = flowBand(r.saturation, r.flow, r.bottleneck);
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
          : css(
              band === "bottleneck"
                ? "--flow-crit"
                : band === "under"
                  ? "--flow-warn"
                  : band === "idle"
                    ? "--steel-500"
                    : "--flow-ok",
            );
        ctx.lineWidth = band === "bottleneck" ? 6 : 2;
        // drones read as dotted air routes; the band grammar overrides. Idle
        // (0-flow) routes read as a dim, sparsely-dotted neutral line, not green.
        ctx.setLineDash(
          band === "idle" ? [2, 6] : band === "good" ? (r.kind === "drone" ? [2, 5] : []) : band === "under" ? [10, 5] : [6, 4],
        );
      }
      ctx.stroke();
      ctx.setLineDash([]);

      const glyphColor = ctx.strokeStyle as string;
      const totalLen = pts.reduce((s, p, i) => (i ? s + Math.hypot(p.x - pts[i - 1].x, p.y - pts[i - 1].y) : 0), 0);
      if (totalLen < 24) continue; // too short for notation at this zoom
      // rail: perpendicular crossties, the classic notation
      if (r.kind === "rail") {
        ctx.strokeStyle = glyphColor;
        ctx.lineWidth = 1.5;
        this.alongLine(pts, 26, (x, y, a) => {
          ctx.beginPath();
          ctx.moveTo(x - Math.sin(a) * 4, y + Math.cos(a) * 4);
          ctx.lineTo(x + Math.sin(a) * 4, y - Math.cos(a) * 4);
          ctx.stroke();
        });
      }
      // truck: small cargo squares between the chevrons
      if (r.kind === "truck") {
        ctx.fillStyle = glyphColor;
        this.alongLine(pts, 140, (x, y, a) => {
          ctx.save();
          ctx.translate(x, y);
          ctx.rotate(a);
          ctx.fillRect(-2.5, -2.5, 5, 5);
          ctx.restore();
        });
      }
      // every cargo route: flow-direction chevrons (path order is from → to)
      ctx.strokeStyle = glyphColor;
      ctx.lineWidth = 2;
      const chevronSpacing = Math.min(r.kind === "belt" ? 90 : 140, Math.max(24, totalLen / 2));
      this.alongLine(
        pts,
        chevronSpacing,
        (x, y, a) => {
          ctx.save();
          ctx.translate(x, y);
          ctx.rotate(a);
          ctx.beginPath();
          ctx.moveTo(-3.5, -4);
          ctx.lineTo(3.5, 0);
          ctx.lineTo(-3.5, 4);
          ctx.stroke();
          ctx.restore();
        },
        r.kind === "truck" ? chevronSpacing / 2 : 0, // interleave with squares
      );
    }
    // pass 2: label chips, selected first, overlaps culled
    const byPriority = [...this.data.routes].sort((a, b) => Number(b.selected) - Number(a.selected));
    for (const r of byPriority) {
      const pts = this.lanePts(r.path.map((p) => map.latLngToContainerPoint(toLatLng(p))), r.lane, r.lanes);
      if (pts.length < 2) continue;
      const band = flowBand(r.saturation, r.flow, r.bottleneck);
      const mid = pts[Math.floor((pts.length - 1) / 2)];
      const mid2 = pts[Math.min(pts.length - 1, Math.floor((pts.length - 1) / 2) + 1)];
      const cx = (mid.x + mid2.x) / 2;
      const cy = (mid.y + mid2.y) / 2;
      // computed transport throughputs are long floats — round for the chip
      const fmtN = (x: number) => (x >= 100 ? Math.round(x) : Math.round(x * 100) / 100);
      const text = `${r.itemName} · ${fmtN(r.flow)}/${fmtN(r.capacity)} · ${Math.round(
        r.saturation * 100,
      )}%  ${r.tag}`;
      ctx.font = `italic 500 9px ${css("--font-mono")}`;
      const w = ctx.measureText(text).width + 10;
      const bg = band === "bottleneck" ? css("--flow-crit") : css("--steel-800");
      const border = r.selected ? css("--signal-500") : css("--steel-600");
      const ink =
        band === "bottleneck"
          ? css("--on-signal")
          : band === "under"
            ? css("--flow-warn")
            : band === "idle"
              ? css("--ink-500")
              : css("--bp-400");
      if (this.placeChip(cx - w / 2, cy - 8, w, 16)) {
        ctx.fillStyle = bg;
        ctx.strokeStyle = border;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.rect(cx - w / 2, cy - 8, w, 16);
        ctx.fill();
        ctx.stroke();
        ctx.fillStyle = ink;
        ctx.textAlign = "center";
        ctx.fillText(text, cx, cy + 3);
        ctx.textAlign = "left";
        continue;
      }
      // degrade, don't vanish: a tag-only micro-chip (MK.2 / RAIL / …) keeps
      // the transport level readable where the full label can't fit
      ctx.font = `500 8px ${css("--font-mono")}`;
      const mw = ctx.measureText(r.tag).width + 8;
      if (!this.placeChip(cx - mw / 2, cy - 6, mw, 12)) continue;
      ctx.fillStyle = bg;
      ctx.strokeStyle = border;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.rect(cx - mw / 2, cy - 6, mw, 12);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = ink;
      ctx.textAlign = "center";
      ctx.fillText(r.tag, cx, cy + 3);
      ctx.textAlign = "left";
    }
  }

  /** Power lines: single 2px line; the chip carries the circuit margin, not
   *  link load — power is a bus, not a belt (A2.1). */
  private drawPower(ctx: CanvasRenderingContext2D, map: L.Map) {
    for (const l of this.data.powerLines) {
      const [a, b] = this.lanePts(
        [map.latLngToContainerPoint(toLatLng(l.from)), map.latLngToContainerPoint(toLatLng(l.to))],
        l.lane,
        l.lanes,
      );
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
        if (!this.placeChip(p.x - w / 2, p.y + 12, w, 15)) continue;
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
      if (!this.placeChip(p.x - w / 2, p.y - 10, w, 20)) continue;
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
      const [a, b] = this.lanePts(
        [map.latLngToContainerPoint(toLatLng(l.from)), map.latLngToContainerPoint(toLatLng(l.to))],
        l.lane,
        l.lanes,
      );
      if (distToSegment(point, a, b) < 8) return l.id;
    }
    return null;
  }

  /** Claim tethers: a quiet dashed line from each claimed node to the pin
   *  that owns it; selection/hover promotes it to signal and names the
   *  factory at the midpoint. Conflicted claims read crit. */
  private drawClaimLinks(ctx: CanvasRenderingContext2D, map: L.Map) {
    const signal = css("--signal-500");
    for (const link of this.data.claimLinks) {
      const a = map.latLngToContainerPoint(toLatLng(link.node));
      const b = map.latLngToContainerPoint(toLatLng(link.factory));
      const color = link.conflict
        ? css("--flow-crit")
        : link.planned && !link.highlight
          ? css("--bp-400")
          : signal;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.setLineDash([4, 4]);
      ctx.strokeStyle = color;
      // Glow so the claim path reads over the busy map — strong when the node
      // or its factory is selected/hovered (trace what this factory uses), a
      // soft hint at rest so the paths are never invisible dim-steel again.
      ctx.shadowColor = color;
      ctx.shadowBlur = link.highlight ? 9 : 4;
      ctx.lineWidth = link.highlight ? 2 : 1.25;
      ctx.globalAlpha = link.highlight ? 1 : 0.7;
      ctx.stroke();
      // crisp core with the glow off, so the dash stays sharp and the blur
      // never bleeds into the nodes drawn next
      ctx.shadowBlur = 0;
      ctx.shadowColor = "rgba(0,0,0,0)";
      ctx.stroke();
      ctx.globalAlpha = 1;
      ctx.setLineDash([]);
      if (link.highlight) {
        const text = `→ ${link.factoryName.toUpperCase()}`;
        ctx.font = `500 9px ${css("--font-mono")}`;
        const w = ctx.measureText(text).width + 10;
        const cx = (a.x + b.x) / 2;
        const cy = (a.y + b.y) / 2;
        ctx.fillStyle = css("--steel-800");
        ctx.strokeStyle = css("--steel-600");
        ctx.lineWidth = 1;
        ctx.fillRect(cx - w / 2, cy - 8, w, 15);
        ctx.strokeRect(cx - w / 2, cy - 8, w, 15);
        ctx.fillStyle = css("--ink-300");
        ctx.textAlign = "center";
        ctx.fillText(text, cx, cy + 3);
        ctx.textAlign = "left";
      }
    }
  }

  /** Refactor tethers (W2a): a steel/blueprint-dashed line from each retiring
   *  ◆ factory to its ◇ replacement — "this replaces that" made visible.
   *  "Orange is a verb": the line rests as blueprint dash and promotes to signal
   *  orange (named REPLACES at the midpoint) only when either pin is selected. */
  private drawReplacesLinks(ctx: CanvasRenderingContext2D, map: L.Map) {
    for (const link of this.data.replacesLinks) {
      const a = map.latLngToContainerPoint(toLatLng(link.old));
      const b = map.latLngToContainerPoint(toLatLng(link.new));
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = link.highlight ? css("--signal-500") : css("--bp-400");
      ctx.lineWidth = link.highlight ? 2 : 1.25;
      ctx.setLineDash([6, 4]);
      ctx.stroke();
      ctx.setLineDash([]);
      if (link.highlight) {
        const text = "REPLACES";
        ctx.font = `600 9px ${css("--font-mono")}`;
        const w = ctx.measureText(text).width + 10;
        const cx = (a.x + b.x) / 2;
        const cy = (a.y + b.y) / 2;
        ctx.fillStyle = css("--steel-800");
        ctx.strokeStyle = css("--signal-500");
        ctx.lineWidth = 1;
        ctx.fillRect(cx - w / 2, cy - 8, w, 15);
        ctx.strokeRect(cx - w / 2, cy - 8, w, 15);
        ctx.fillStyle = css("--signal-500");
        ctx.textAlign = "center";
        ctx.fillText(text, cx, cy + 3);
        ctx.textAlign = "left";
      }
    }
  }

  /** Fan parallel routes: offset projected points perpendicular to the
   *  overall segment by lane — screen-space, so the separation is
   *  zoom-stable and hit-tests share it. */
  private lanePts(pts: L.Point[], lane: number, lanes: number): L.Point[] {
    if (lanes <= 1 || pts.length < 2) return pts;
    const a = pts[0];
    const b = pts[pts.length - 1];
    const dx = b.x - a.x;
    const dy = b.y - a.y;
    const len = Math.hypot(dx, dy) || 1;
    const off = (lane - (lanes - 1) / 2) * 9;
    const ox = (-dy / len) * off;
    const oy = (dx / len) * off;
    return pts.map((p) => L.point(p.x + ox, p.y + oy));
  }

  /** Reserve a chip rect; false = an earlier chip owns this space. */
  private placeChip(x: number, y: number, w: number, h: number): boolean {
    const pad = 2;
    const hit = this.chipRects.some(
      (k) => x - pad < k.x + k.w && k.x < x + w + pad && y - pad < k.y + k.h && k.y < y + h + pad,
    );
    if (!hit) this.chipRects.push({ x, y, w, h });
    return !hit;
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
      const pts = this.lanePts(r.path.map((p) => map.latLngToContainerPoint(toLatLng(p))), r.lane, r.lanes);
      for (let i = 0; i < pts.length - 1; i++) {
        if (distToSegment(point, pts[i], pts[i + 1]) < 8) return r.id;
      }
    }
    return null;
  }

  /** Decode the terrain render once, bake the muted filter in, then redraw.
   *  Drawing on THIS canvas (not a Leaflet pane) keeps stacking trivial: the
   *  map-pane (z 400) with its DOM pins stays above the whole canvas (z 200),
   *  and terrain/grid/nodes move in lockstep on every pan/zoom frame. */
  private async loadTerrain() {
    if (this.terrainCanvas || this.terrainLoading) return;
    this.terrainLoading = true;
    try {
      const blob = await (await fetch(TERRAIN_URL)).blob();
      // createImageBitmap decodes off the main thread — the sync <img> decode
      // (~850 ms for the 5000² render) stalled map init and marker placement
      const bmp = await createImageBitmap(blob);
      const off = document.createElement("canvas");
      off.width = bmp.width;
      off.height = bmp.height;
      const octx = off.getContext("2d")!;
      octx.filter = TERRAIN_FILTER;
      octx.drawImage(bmp, 0, 0);
      bmp.close();
      this.terrainCanvas = off;
      this.redraw();
    } catch {
      // terrain stays off — the flat survey canvas is the honest fallback
    }
  }

  /** Blit the visible slice of the terrain render under everything else. */
  private drawTerrain(ctx: CanvasRenderingContext2D, map: L.Map, size: L.Point) {
    const src = this.terrainCanvas;
    if (!src) return;
    const tl = map.latLngToContainerPoint(toLatLng({ x: TERRAIN_BOUNDS.minX, y: TERRAIN_BOUNDS.minY }));
    const br = map.latLngToContainerPoint(toLatLng({ x: TERRAIN_BOUNDS.maxX, y: TERRAIN_BOUNDS.maxY }));
    const scaleX = (br.x - tl.x) / src.width;
    const scaleY = (br.y - tl.y) / src.height;
    // clamp to the viewport so deep zooms never ask for a giant dest rect
    const dx0 = Math.max(tl.x, 0);
    const dy0 = Math.max(tl.y, 0);
    const dx1 = Math.min(br.x, size.x);
    const dy1 = Math.min(br.y, size.y);
    if (dx1 <= dx0 || dy1 <= dy0) return;
    ctx.drawImage(
      src,
      (dx0 - tl.x) / scaleX,
      (dy0 - tl.y) / scaleY,
      (dx1 - dx0) / scaleX,
      (dy1 - dy0) / scaleY,
      dx0,
      dy0,
      dx1 - dx0,
      dy1 - dy0,
    );
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
    // hoisted out of the per-node loop — one lookup per redraw, not per node
    const fontMono = css("--font-mono");
    const ink100 = css("--ink-100");
    // Resource identity fill (map data, not a UI signal): read type at a glance.
    // Palette resolved once per redraw, indexed by extracted resource class.
    const resourceGeneric = css("--resource-generic");
    const resourceFill: Record<string, string> = {
      Desc_OreIron_C: css("--resource-iron"),
      Desc_OreCopper_C: css("--resource-copper"),
      Desc_Stone_C: css("--resource-limestone"),
      Desc_Coal_C: css("--resource-coal"),
      Desc_OreGold_C: css("--resource-caterium"),
      Desc_RawQuartz_C: css("--resource-quartz"),
      Desc_Sulfur_C: css("--resource-sulfur"),
      Desc_LiquidOil_C: css("--resource-oil"),
      Desc_OreBauxite_C: css("--resource-bauxite"),
      Desc_OreUranium_C: css("--resource-uranium"),
      Desc_SAM_C: css("--resource-sam"),
    };

    // 459 real nodes at world zoom are a wall of rings — dots shrink as the
    // view widens so factories and flows stay the foreground layer
    const zoom = map.getZoom();
    const rBase = zoom <= 2 ? 3.5 : zoom <= 3 ? 5 : 7;

    const filter = this.data.nodeFilter;
    this.nodeScreen = [];
    for (const node of this.data.world.nodes) {
      // Search filter: hide (skip drawing AND hit-testing) non-matching nodes.
      if (filter?.active && !filter.visible.has(node.id)) continue;
      const state = this.data.nodeStates[node.id] ?? { claims: 0, conflict: false, claimed: false };
      const p = map.latLngToContainerPoint(toLatLng(node));
      this.nodeScreen.push({ node, x: p.x, y: p.y });
      const hovered = this.data.hoveredNode === node.id;
      const selected = this.data.selectedNode === node.id;
      const r = hovered || selected ? 7 : rBase;

      // halo so nodes read over the grid
      ctx.beginPath();
      ctx.arc(p.x, p.y, r + 3, 0, Math.PI * 2);
      ctx.fillStyle = canvasBg;
      ctx.fill();

      // resource-identity fill: the disc is tinted by extracted type so iron /
      // copper / oil read apart at a glance. Free nodes sit a touch dimmer than
      // claimed so the orange claim dot still leads the eye.
      ctx.beginPath();
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
      ctx.globalAlpha = state.claimed ? 1 : 0.72;
      ctx.fillStyle = resourceFill[node.item] ?? resourceGeneric;
      ctx.fill();
      ctx.globalAlpha = 1;

      // Claimed nodes (a factory is extracting here) get a soft signal halo so
      // "in use" reads at a glance against the wall of free nodes.
      if (state.claimed && !state.conflict && !selected) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 3, 0, Math.PI * 2);
        ctx.lineWidth = 3;
        ctx.strokeStyle = signal;
        ctx.globalAlpha = 0.4;
        ctx.stroke();
        ctx.globalAlpha = 1;
      }

      // purity ring: pure solid / normal dashed / impure dotted. Colour also
      // carries claim status — a claimed node's ring is signal-orange, not the
      // dim ink of a free node.
      ctx.beginPath();
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
      const ringW = hovered || selected || state.claimed ? 2 : 1.5;
      if (node.purity === "normal") ctx.setLineDash([4, 3]);
      else if (node.purity === "impure") ctx.setLineDash([1.5, 2.5]);
      else ctx.setLineDash([]);
      if (state.conflict || state.claimed) {
        // canvas-bg under-stroke keyline (same dash pattern, drawn first) so
        // the coloured dashes keep an edge over any resource fill
        ctx.lineWidth = ringW + 2;
        ctx.strokeStyle = canvasBg;
        ctx.stroke();
      }
      ctx.lineWidth = ringW;
      ctx.strokeStyle = state.conflict
        ? crit
        : hovered || selected
          ? ink100
          : state.claimed
            ? signal
            : inkMuted;
      ctx.stroke();
      ctx.setLineDash([]);

      // cave nodes: an under-arc below the ring (underground), plus the
      // surface entrance — a small square (infrastructure, A2.3) linked by a
      // dotted line while hovered/selected so routing via it reads naturally
      if (node.zone === "cave") {
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 3.5, Math.PI * 0.15, Math.PI * 0.85);
        ctx.lineWidth = 1.5;
        ctx.strokeStyle = hovered || selected ? ink100 : inkMuted;
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
          ctx.font = `500 9px ${fontMono}`;
          ctx.fillStyle = inkMuted;
          ctx.fillText("ENTRANCE", e.x + 7, e.y + 3);
        }
      }

      // claimed = orange center dot; free = hollow
      if (state.claimed) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, Math.min(3, r - 1.5), 0, Math.PI * 2);
        ctx.fillStyle = state.conflict ? crit : signal;
        ctx.fill();
        // 1px canvas-bg outline on the same path: the dot keeps an edge on
        // resource fills whose luminance sits near the mark's (gold, green)
        ctx.lineWidth = 1;
        ctx.strokeStyle = canvasBg;
        ctx.stroke();
      }

      // W2b-C drift marker: a small hollow diamond off the ring when the node
      // sits at a plan-corrected (save-reconciled) position.
      if (state.drift) {
        const dx = p.x + r + 3;
        const dy = p.y - r - 1;
        ctx.beginPath();
        ctx.moveTo(dx, dy - 3);
        ctx.lineTo(dx + 3, dy);
        ctx.lineTo(dx, dy + 3);
        ctx.lineTo(dx - 3, dy);
        ctx.closePath();
        ctx.lineWidth = 1.25;
        ctx.strokeStyle = signal;
        ctx.stroke();
      }

      // mono label under every node (mock 2a: FE PURE #08), on a dark plate so
      // it stays legible over the terrain/grid instead of washing out as bare
      // text — culled when it would overlap an earlier chip/label (hover/select
      // always wins); at world zoom the dots speak for themselves
      if (zoom > 2 || hovered || selected || state.conflict) {
        ctx.font = `500 9px ${fontMono}`;
        const label = this.nodeLabel(node);
        const lw = ctx.measureText(label).width;
        if (hovered || selected || this.placeChip(p.x - lw / 2, p.y + r + 3, lw, 12)) {
          const ly = p.y + r + 12;
          const padX = 4;
          ctx.fillStyle = "rgba(9, 12, 16, 0.76)";
          ctx.beginPath();
          ctx.roundRect(p.x - lw / 2 - padX, ly - 10, lw + padX * 2, 13, 3);
          ctx.fill();
          ctx.textAlign = "center";
          ctx.fillStyle = state.conflict ? crit : state.claimed ? signal : ink100;
          ctx.fillText(label, p.x, ly);
          ctx.textAlign = "left";
        }
      }

      if (state.conflict) {
        ctx.font = `700 9px ${fontMono}`;
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
