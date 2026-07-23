// Belt router: parallel belts must land in DISTINCT vertical lanes so a bundle
// reads as several belts, not one thick line — while a belt with no conflict
// keeps its natural midpoint (no gratuitous zig-zag).

import { describe, expect, it } from "vitest";
import { computeEdgeLayout, type NodeGeom, type EdgeIn } from "./edgeLayout";

// x of the vertical turn segment of a plain forward belt (points[1] and [2]
// share it): route() emits [s, {turnX, s.y}, {turnX, t.y}, t].
const turnX = (geom: { points: { x: number; y: number }[] }) => geom.points[1]?.x;

describe("edgeLayout lane separation", () => {
  it("splits two overlapping parallel belts into separate lanes", () => {
    // Two belts crossing the SAME corridor with overlapping y-spans — without
    // lanes their vertical segments sit on the identical x line (look like one).
    const nodes: Record<string, NodeGeom> = {
      A: { x: 0, y: 0, w: 100, h: 40 },
      B: { x: 0, y: 200, w: 100, h: 40 },
      T1: { x: 300, y: 0, w: 100, h: 40 },
      T2: { x: 300, y: 200, w: 100, h: 40 },
    };
    const edges: EdgeIn[] = [
      { id: "a-t2", source: "A", target: "T2" }, // top → bottom
      { id: "b-t1", source: "B", target: "T1" }, // bottom → top
    ];
    const geom = computeEdgeLayout(nodes, edges);
    const x1 = turnX(geom["a-t2"]);
    const x2 = turnX(geom["b-t1"]);
    expect(Math.abs(x1 - x2)).toBeGreaterThanOrEqual(16); // LANE_GAP
  });

  it("leaves a lone belt on its natural midpoint", () => {
    const nodes: Record<string, NodeGeom> = {
      A: { x: 0, y: 0, w: 100, h: 40 },
      B: { x: 300, y: 120, w: 100, h: 40 },
    };
    const geom = computeEdgeLayout(nodes, [{ id: "a-b", source: "A", target: "B" }]);
    // midpoint between A's right face (x=100) and B's left face (x=300) = 200
    expect(turnX(geom["a-b"])).toBe(200);
  });

  it("keeps parallel belts at different heights on their base lane (no overlap)", () => {
    // Vertical spans that don't overlap in y must NOT be pushed apart in x.
    const nodes: Record<string, NodeGeom> = {
      A: { x: 0, y: 0, w: 100, h: 40 },
      B: { x: 0, y: 200, w: 100, h: 40 },
      T1: { x: 300, y: 60, w: 100, h: 40 },
      T2: { x: 300, y: 260, w: 100, h: 40 },
    };
    const edges: EdgeIn[] = [
      { id: "a-t1", source: "A", target: "T1" },
      { id: "b-t2", source: "B", target: "T2" },
    ];
    const geom = computeEdgeLayout(nodes, edges);
    expect(turnX(geom["a-t1"])).toBe(200);
    expect(turnX(geom["b-t2"])).toBe(200);
  });
});
