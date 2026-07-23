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

/** Which face a belt leaves/enters, so junctions can route to distinct sides
 *  the way the in-game splitter/merger do (splitter: 1 in ← left, 3 out →
 *  top/right/bottom; merger mirrors it). Non-junction cards always use L→out,
 *  R→in as before. */
type Dir = "L" | "R" | "T" | "B";
interface Anchor extends Pt {
  dir: Dir;
}
/** Junction shapes that get game-accurate side orientation. Storage and any
 *  other kind fall through to the default right-out / left-in behaviour. */
export type JunctionShape = "splitter" | "merger" | "cross";

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
  /** total polyline length — short belts render compact labels */
  pathLen: number;
}

export interface LabelSize {
  w: number;
  h: number;
}

const STUB = 24; // straight run leaving/entering a card
const CORNER_R = 8; // belt curve radius
const HOP_R = 6; // crossing bridge radius
const LANE_GAP = 16; // min horizontal separation between parallel belt lanes

/** Point on a node face `dir` at fractional offset `frac` along that face. */
function faceAnchor(n: NodeGeom, dir: Dir, frac = 0.5): Anchor {
  switch (dir) {
    case "L":
      return { x: n.x, y: n.y + n.h * frac, dir };
    case "R":
      return { x: n.x + n.w, y: n.y + n.h * frac, dir };
    case "T":
      return { x: n.x + n.w * frac, y: n.y, dir };
    case "B":
      return { x: n.x + n.w * frac, y: n.y + n.h, dir };
  }
}

// Which faces the "many" side of a splitter/merger uses, by count — one belt
// per side, mirroring the real building (opposite side + the two perpendicular
// sides). Sorted by counterpart Y so the topmost belt takes the top face.
const OUT_FACES: Record<number, Dir[]> = { 1: ["R"], 2: ["T", "B"], 3: ["T", "R", "B"] };
const IN_FACES: Record<number, Dir[]> = { 1: ["L"], 2: ["T", "B"], 3: ["T", "L", "B"] };
// The Pipeline Junction Cross both merges AND splits, so it fans BOTH sides: up
// to 2 inputs on its left/top faces and up to 2 outputs on its right/bottom
// faces — the 2-in/2-out orientation its four handles imply. (Beyond 2 on a
// side, it falls through to the even spread, same as any junction.)
const CROSS_IN_FACES: Record<number, Dir[]> = { 1: ["L"], 2: ["L", "T"] };
const CROSS_OUT_FACES: Record<number, Dir[]> = { 1: ["R"], 2: ["R", "B"] };

/** Deterministic anchors: edges sorted by counterpart center-Y (ties by id),
 *  spread evenly along the node face. Splitters/mergers instead fan their
 *  many-side belts across distinct faces (game orientation). */
function anchorPositions(
  nodes: Record<string, NodeGeom>,
  edges: EdgeIn[],
  shapes: Record<string, JunctionShape>,
): { src: Record<string, Anchor>; dst: Record<string, Anchor> } {
  const src: Record<string, Anchor> = {};
  const dst: Record<string, Anchor> = {};
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
    const shape = shapes[nodeId];
    list.sort((a, b) => centerY(a.target) - centerY(b.target) || a.id.localeCompare(b.id));
    if (shape === "cross" && CROSS_OUT_FACES[list.length]) {
      // a pipe cross fans its outputs across the right/bottom faces
      const faces = CROSS_OUT_FACES[list.length];
      list.forEach((e, i) => (src[e.id] = faceAnchor(n, faces[i])));
    } else if (shape === "splitter" && OUT_FACES[list.length]) {
      // outputs fan across distinct faces
      const faces = OUT_FACES[list.length];
      list.forEach((e, i) => (src[e.id] = faceAnchor(n, faces[i])));
    } else if (shape === "merger" && list.length === 1) {
      // the single merged output leaves the right face
      src[list[0].id] = faceAnchor(n, "R");
    } else {
      list.forEach((e, i) => {
        src[e.id] = { x: n.x + n.w, y: n.y + (n.h * (i + 1)) / (list.length + 1), dir: "R" };
      });
    }
  }
  for (const [nodeId, list] of byTarget) {
    const n = nodes[nodeId];
    const shape = shapes[nodeId];
    list.sort((a, b) => centerY(a.source) - centerY(b.source) || a.id.localeCompare(b.id));
    if (shape === "cross" && CROSS_IN_FACES[list.length]) {
      // a pipe cross fans its inputs across the left/top faces
      const faces = CROSS_IN_FACES[list.length];
      list.forEach((e, i) => (dst[e.id] = faceAnchor(n, faces[i])));
    } else if (shape === "merger" && IN_FACES[list.length]) {
      // inputs fan across distinct faces
      const faces = IN_FACES[list.length];
      list.forEach((e, i) => (dst[e.id] = faceAnchor(n, faces[i])));
    } else if (shape === "splitter" && list.length === 1) {
      // the single feed enters the left face
      dst[list[0].id] = faceAnchor(n, "L");
    } else {
      list.forEach((e, i) => {
        dst[e.id] = { x: n.x, y: n.y + (n.h * (i + 1)) / (list.length + 1), dir: "L" };
      });
    }
  }
  return { src, dst };
}

const isH = (d: Dir) => d === "L" || d === "R";
function step(a: Anchor, d: number): Pt {
  switch (a.dir) {
    case "L":
      return { x: a.x - d, y: a.y };
    case "R":
      return { x: a.x + d, y: a.y };
    case "T":
      return { x: a.x, y: a.y - d };
    case "B":
      return { x: a.x, y: a.y + d };
  }
}

/** Orthogonal belt honouring each anchor's face direction: leave along the
 *  face normal (a STUB), then turn once or twice to reach the other stub. Used
 *  for any belt touching an oriented junction — the extra faces mean the plain
 *  right→left router can't express a top/bottom departure. */
function orthoRoute(s: Anchor, t: Anchor): Pt[] {
  const s1 = step(s, STUB);
  const t1 = step(t, STUB);
  const mids: Pt[] = [];
  const sH = isH(s.dir);
  const tH = isH(t.dir);
  if (sH && tH) {
    if (Math.abs(s1.y - t1.y) >= 1) {
      const midX = Math.round((s1.x + t1.x) / 2);
      mids.push({ x: midX, y: s1.y }, { x: midX, y: t1.y });
    }
  } else if (!sH && !tH) {
    if (Math.abs(s1.x - t1.x) >= 1) {
      const midY = Math.round((s1.y + t1.y) / 2);
      mids.push({ x: s1.x, y: midY }, { x: t1.x, y: midY });
    }
  } else if (sH && !tH) {
    mids.push({ x: t1.x, y: s1.y });
  } else {
    mids.push({ x: s1.x, y: t1.y });
  }
  return [{ x: s.x, y: s.y }, s1, ...mids, t1, { x: t.x, y: t.y }];
}

/** Assign each plain forward belt the x of its vertical turn segment, spreading
 *  belts that would otherwise stack onto ONE line into distinct parallel lanes.
 *  Greedy + deterministic: process by base turn-x then span; each belt keeps its
 *  natural midpoint unless that lane already carries a vertical segment
 *  overlapping in y (within LANE_GAP in x), in which case it steps outward
 *  (±LANE_GAP…) to the nearest free lane inside the corridor between the two
 *  cards. Only belts that actually conflict move — a lone belt keeps its midX. */
function assignForwardLanes(
  edges: EdgeIn[],
  src: Record<string, Anchor>,
  dst: Record<string, Anchor>,
  oriented: (e: EdgeIn) => boolean,
): Record<string, number> {
  const turn: Record<string, number> = {};
  const fwd = edges
    .filter(
      (e) =>
        src[e.id] &&
        dst[e.id] &&
        !oriented(e) &&
        dst[e.id].x > src[e.id].x + 4 &&
        Math.abs(dst[e.id].y - src[e.id].y) >= 1,
    )
    .map((e) => {
      const s = src[e.id];
      const t = dst[e.id];
      return {
        id: e.id,
        base: Math.round((s.x + t.x) / 2),
        y1: Math.min(s.y, t.y),
        y2: Math.max(s.y, t.y),
        lo: Math.min(s.x, t.x) + 6, // keep the lane between the two faces
        hi: Math.max(s.x, t.x) - 6,
      };
    })
    .sort((a, b) => a.base - b.base || a.y1 - b.y1 || a.id.localeCompare(b.id));

  const placed: { x: number; y1: number; y2: number }[] = [];
  const conflicts = (x: number, y1: number, y2: number) =>
    placed.some((p) => Math.abs(p.x - x) < LANE_GAP && y1 < p.y2 - 0.5 && y2 > p.y1 + 0.5);

  for (const e of fwd) {
    let x = e.base;
    if (conflicts(x, e.y1, e.y2)) {
      for (let k = 1; k < 64; k++) {
        const cand = [e.base + k * LANE_GAP, e.base - k * LANE_GAP].filter((c) => c >= e.lo && c <= e.hi);
        const free = cand.find((c) => !conflicts(c, e.y1, e.y2));
        if (free !== undefined) {
          x = free;
          break;
        }
        if (cand.length === 0) break; // corridor exhausted — keep the base lane
      }
    }
    turn[e.id] = x;
    placed.push({ x, y1: e.y1, y2: e.y2 });
  }
  return turn;
}

/** Axis-aligned polyline from source anchor to target anchor. `turnX` is the
 *  lane-assigned x of the vertical segment (defaults to the midpoint). */
function route(s: Pt, t: Pt, srcNode: NodeGeom, dstNode: NodeGeom, turnX?: number): Pt[] {
  if (t.x > s.x + 4) {
    // any forward progress: simple H-V-H with the turn at its assigned lane
    const midX = turnX ?? Math.round((s.x + t.x) / 2);
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

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
}

function pathLength(points: Pt[]): number {
  let len = 0;
  for (let i = 0; i < points.length - 1; i++) {
    len += Math.abs(points[i + 1].x - points[i].x) + Math.abs(points[i + 1].y - points[i].y);
  }
  return len;
}

/** Collision-aware chip placement: try several spots along the belt (preferring
 *  long horizontal runs), rejecting any that overlap a card or an already
 *  placed chip. Chips are the always-present data channel (mock 1e) — they must
 *  stay readable when machines crowd together. */
function placeLabel(points: Pt[], size: LabelSize, obstacles: Rect[], placed: Rect[]): Pt {
  const segs = segments(points);
  const ranked = segs
    .map((seg) => ({ seg, len: Math.abs(seg.b.x - seg.a.x) + Math.abs(seg.b.y - seg.a.y) }))
    .sort((a, b) => (Number(b.seg.horizontal) - Number(a.seg.horizontal)) * 1000 + (b.len - a.len));

  const candidates: Pt[] = [];
  for (const { seg } of ranked) {
    for (const t of [0.5, 0.35, 0.65, 0.2, 0.8]) {
      candidates.push({ x: seg.a.x + (seg.b.x - seg.a.x) * t, y: seg.a.y + (seg.b.y - seg.a.y) * t });
    }
  }

  const rectFor = (c: Pt): Rect => ({ x: c.x - size.w / 2, y: c.y - size.h / 2, w: size.w, h: size.h });
  let fallback = candidates[0] ?? points[0];
  let fallbackScore = Infinity;
  for (const c of candidates) {
    const r = rectFor(c);
    const hitsCard = obstacles.some((o) => overlaps(r, o));
    const hitsChip = placed.some((o) => overlaps(r, o));
    if (!hitsCard && !hitsChip) {
      placed.push(r);
      return c;
    }
    // score fallbacks: card overlap is worse than chip overlap
    const score = (hitsCard ? 2 : 0) + (hitsChip ? 1 : 0);
    if (score < fallbackScore) {
      fallbackScore = score;
      fallback = c;
    }
  }
  placed.push(rectFor(fallback));
  return fallback;
}

export function computeEdgeLayout(
  nodes: Record<string, NodeGeom>,
  edges: EdgeIn[],
  labelSizes: Record<string, LabelSize> = {},
  shapes: Record<string, JunctionShape> = {},
): Record<string, EdgeGeom> {
  const usable = edges.filter((e) => nodes[e.source] && nodes[e.target]);
  const { src, dst } = anchorPositions(nodes, usable, shapes);
  const isOriented = (e: EdgeIn) => !!shapes[e.source] || !!shapes[e.target];
  // Lane-separate parallel belts' vertical segments so they don't merge into one.
  const lanes = assignForwardLanes(usable, src, dst, isOriented);
  const polylines = usable.map((e) => {
    // Any belt touching an oriented junction uses the face-aware router so a
    // top/bottom departure reads correctly; everything else keeps the plain
    // right→left belt run (with its lane-assigned vertical turn).
    const points = isOriented(e)
      ? orthoRoute(src[e.id], dst[e.id])
      : route(src[e.id], dst[e.id], nodes[e.source], nodes[e.target], lanes[e.id]);
    return { id: e.id, points: dedupe(points) };
  });
  const hops = findHops(polylines);

  // Cards (slightly inflated) are obstacles; chips also avoid one another.
  const obstacles: Rect[] = Object.values(nodes).map((n) => ({
    x: n.x - 4,
    y: n.y - 4,
    w: n.w + 8,
    h: n.h + 8,
  }));
  const placed: Rect[] = [];

  // Place labels for the longest belts first — short belts have fewer options
  // and will dodge around the chips that matter most.
  const byLen = [...polylines].sort((a, b) => pathLength(b.points) - pathLength(a.points));
  const labels: Record<string, Pt> = {};
  for (const p of byLen) {
    const size = labelSizes[p.id] ?? { w: 120, h: 20 };
    labels[p.id] = placeLabel(p.points, size, obstacles, placed);
  }

  const out: Record<string, EdgeGeom> = {};
  for (const p of polylines) {
    out[p.id] = {
      points: p.points,
      hops: hops[p.id] ?? [],
      path: buildPath(p.points, hops[p.id] ?? []),
      labelX: labels[p.id].x,
      labelY: labels[p.id].y,
      pathLen: pathLength(p.points),
    };
  }
  return out;
}
