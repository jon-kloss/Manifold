// Belt-style edge layout. Conveyors are aesthetic objects in this game, so
// edges behave like belts, not wires: they leave the right face and enter the
// left face at evenly spread, deterministically ordered anchor points, run in
// axis-aligned segments with rounded corners, and *hop* over belts they cross
// (schematic bridge) so crossings read as intentional.

export interface NodeGeom {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface EdgeIn {
  id: string;
  source: string;
  target: string;
}

interface Pt {
  x: number;
  y: number;
}

interface Hop {
  /** index of the segment (between points[i] and points[i+1]) */
  seg: number;
  /** coordinate along the segment's moving axis where the crossing sits */
  at: number;
}

export interface EdgeGeom {
  points: Pt[];
  hops: Hop[];
  path: string;
  labelX: number;
  labelY: number;
}

const STUB = 24; // straight run leaving/entering a card
const CORNER_R = 8; // belt curve radius
const HOP_R = 6; // crossing bridge radius

/** Deterministic anchors: edges sorted by counterpart center-Y (ties by id),
 *  spread evenly along the node face. */
function anchorPositions(
  nodes: Record<string, NodeGeom>,
  edges: EdgeIn[],
): { src: Record<string, Pt>; dst: Record<string, Pt> } {
  const src: Record<string, Pt> = {};
  const dst: Record<string, Pt> = {};
  const bySource = new Map<string, EdgeIn[]>();
  const byTarget = new Map<string, EdgeIn[]>();
  for (const e of edges) {
    if (!nodes[e.source] || !nodes[e.target]) continue;
    bySource.set(e.source, [...(bySource.get(e.source) ?? []), e]);
    byTarget.set(e.target, [...(byTarget.get(e.target) ?? []), e]);
  }
  const centerY = (id: string) => nodes[id].y + nodes[id].h / 2;
  for (const [nodeId, list] of bySource) {
    const n = nodes[nodeId];
    list.sort((a, b) => centerY(a.target) - centerY(b.target) || a.id.localeCompare(b.id));
    list.forEach((e, i) => {
      src[e.id] = { x: n.x + n.w, y: n.y + (n.h * (i + 1)) / (list.length + 1) };
    });
  }
  for (const [nodeId, list] of byTarget) {
    const n = nodes[nodeId];
    list.sort((a, b) => centerY(a.source) - centerY(b.source) || a.id.localeCompare(b.id));
    list.forEach((e, i) => {
      dst[e.id] = { x: n.x, y: n.y + (n.h * (i + 1)) / (list.length + 1) };
    });
  }
  return { src, dst };
}

/** Axis-aligned polyline from source anchor to target anchor. */
function route(s: Pt, t: Pt, srcNode: NodeGeom, dstNode: NodeGeom): Pt[] {
  if (t.x > s.x + 4) {
    // any forward progress: simple H-V-H with the turn at the midpoint
    const midX = Math.round((s.x + t.x) / 2);
    if (Math.abs(t.y - s.y) < 1) return [s, t]; // straight run
    return [s, { x: midX, y: s.y }, { x: midX, y: t.y }, t];
  }
  // target is behind the source: wrap around above or below both cards
  const outX = s.x + STUB;
  const backX = t.x - STUB;
  const top = Math.min(srcNode.y, dstNode.y) - STUB;
  const bottom = Math.max(srcNode.y + srcNode.h, dstNode.y + dstNode.h) + STUB;
  const midY = Math.abs(s.y - top) + Math.abs(t.y - top) <= Math.abs(s.y - bottom) + Math.abs(t.y - bottom) ? top : bottom;
  return [s, { x: outX, y: s.y }, { x: outX, y: midY }, { x: backX, y: midY }, { x: backX, y: t.y }, t];
}

function dedupe(points: Pt[]): Pt[] {
  const out: Pt[] = [];
  for (const p of points) {
    const last = out[out.length - 1];
    if (!last || Math.abs(last.x - p.x) > 0.5 || Math.abs(last.y - p.y) > 0.5) out.push(p);
  }
  // drop collinear middles
  for (let i = out.length - 2; i > 0; i--) {
    const a = out[i - 1];
    const b = out[i];
    const c = out[i + 1];
    if ((a.x === b.x && b.x === c.x) || (a.y === b.y && b.y === c.y)) out.splice(i, 1);
  }
  return out;
}

interface Seg {
  a: Pt;
  b: Pt;
  horizontal: boolean;
}

function segments(points: Pt[]): Seg[] {
  const out: Seg[] = [];
  for (let i = 0; i < points.length - 1; i++) {
    out.push({ a: points[i], b: points[i + 1], horizontal: points[i].y === points[i + 1].y });
  }
  return out;
}

/** Crossings: a later edge hops wherever it crosses an earlier edge. */
function findHops(all: { id: string; points: Pt[] }[]): Record<string, Hop[]> {
  const hops: Record<string, Hop[]> = {};
  const EPS = CORNER_R + HOP_R + 2; // keep hops clear of corners
  for (let j = 1; j < all.length; j++) {
    const later = segments(all[j].points);
    for (let i = 0; i < j; i++) {
      const earlier = segments(all[i].points);
      later.forEach((sj, segIdx) => {
        for (const si of earlier) {
          if (sj.horizontal === si.horizontal) continue;
          const h = sj.horizontal ? sj : si;
          const v = sj.horizontal ? si : sj;
          const hx1 = Math.min(h.a.x, h.b.x);
          const hx2 = Math.max(h.a.x, h.b.x);
          const vy1 = Math.min(v.a.y, v.b.y);
          const vy2 = Math.max(v.a.y, v.b.y);
          const crosses = v.a.x > hx1 + EPS && v.a.x < hx2 - EPS && h.a.y > vy1 + EPS && h.a.y < vy2 - EPS;
          if (!crosses) continue;
          const at = sj.horizontal ? v.a.x : h.a.y;
          (hops[all[j].id] ??= []).push({ seg: segIdx, at });
        }
      });
    }
  }
  return hops;
}

/** SVG path with rounded corners and hop arcs. */
function buildPath(points: Pt[], hops: Hop[]): string {
  const segs = segments(points);
  let d = `M ${points[0].x} ${points[0].y}`;
  segs.forEach((seg, i) => {
    const dirX = Math.sign(seg.b.x - seg.a.x);
    const dirY = Math.sign(seg.b.y - seg.a.y);
    // where this segment actually starts/ends after corner rounding
    const startTrim = i > 0 ? CORNER_R : 0;
    const endTrim = i < segs.length - 1 ? CORNER_R : 0;
    // hops on this segment, ordered along travel direction
    const segHops = hops
      .filter((h) => h.seg === i)
      .map((h) => h.at)
      .sort((a, b) => (seg.horizontal ? (a - b) * dirX : (a - b) * dirY));

    if (i > 0) {
      // corner arc from previous segment into this one (quadratic through the vertex)
      const cornerEnd = seg.horizontal
        ? { x: seg.a.x + dirX * CORNER_R, y: seg.a.y }
        : { x: seg.a.x, y: seg.a.y + dirY * CORNER_R };
      d += ` Q ${seg.a.x} ${seg.a.y} ${cornerEnd.x} ${cornerEnd.y}`;
    }

    const lineTo = (x: number, y: number) => {
      d += ` L ${x} ${y}`;
    };
    if (seg.horizontal) {
      for (const at of segHops) {
        lineTo(at - dirX * HOP_R, seg.a.y);
        // bridge bulges up
        d += ` A ${HOP_R} ${HOP_R} 0 0 ${dirX > 0 ? 1 : 0} ${at + dirX * HOP_R} ${seg.a.y}`;
      }
      lineTo(seg.b.x - dirX * endTrim, seg.b.y);
    } else {
      for (const at of segHops) {
        lineTo(seg.a.x, at - dirY * HOP_R);
        // bridge bulges right
        d += ` A ${HOP_R} ${HOP_R} 0 0 ${dirY > 0 ? 0 : 1} ${seg.a.x} ${at + dirY * HOP_R}`;
      }
      lineTo(seg.b.x, seg.b.y - dirY * endTrim);
    }
    void startTrim;
  });
  return d;
}

function labelPoint(points: Pt[]): Pt {
  // Prefer the longest horizontal run — chips read along the belt and stay
  // clear of card faces; fall back to the longest segment of any kind.
  const segs = segments(points);
  let best: Seg | null = null;
  let bestLen = 0;
  for (const s of segs) {
    const len = Math.abs(s.b.x - s.a.x) + Math.abs(s.b.y - s.a.y);
    const weighted = s.horizontal ? len * 2 : len;
    if (weighted > bestLen) {
      bestLen = weighted;
      best = s;
    }
  }
  const b = best ?? segs[0];
  return { x: (b.a.x + b.b.x) / 2, y: (b.a.y + b.b.y) / 2 };
}

export function computeEdgeLayout(
  nodes: Record<string, NodeGeom>,
  edges: EdgeIn[],
): Record<string, EdgeGeom> {
  const usable = edges.filter((e) => nodes[e.source] && nodes[e.target]);
  const { src, dst } = anchorPositions(nodes, usable);
  const polylines = usable.map((e) => ({
    id: e.id,
    points: dedupe(route(src[e.id], dst[e.id], nodes[e.source], nodes[e.target])),
  }));
  const hops = findHops(polylines);
  const out: Record<string, EdgeGeom> = {};
  for (const p of polylines) {
    const label = labelPoint(p.points);
    out[p.id] = {
      points: p.points,
      hops: hops[p.id] ?? [],
      path: buildPath(p.points, hops[p.id] ?? []),
      labelX: label.x,
      labelY: label.y,
    };
  }
  return out;
}
