// Belt edge with the flow encoding (mock 1e): color + thickness + pattern agree,
// plus the always-present mono label chip `n/cap · %`. Paths are belt-style
// orthogonal runs from edgeLayout (consistent anchors, rounded corners, hop
// arcs over crossed belts). Cross-floor belts are lifts (⇅).

import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { flowLevel } from "../lib/format";
import { fmtRate, fmtPercent } from "../lib/format";
import { beltCapacity, type BeltEdge } from "../state/types";
import type { EdgeGeom } from "./edgeLayout";

export interface BeltEdgeData {
  edge: BeltEdge;
  flow: number;
  saturation: number;
  projected: boolean;
  flowOverlay: boolean;
  settled: boolean;
  geom: EdgeGeom | null;
  lift: boolean;
  dimmed: boolean;
  [key: string]: unknown;
}

const STROKE = {
  ok: { width: 2, dash: undefined, color: "var(--flow-ok)" },
  warn: { width: 4, dash: "10 5", color: "var(--flow-warn)" },
  crit: { width: 6, dash: "6 4", color: "var(--flow-crit)" },
} as const;

export default function BeltEdgeView(props: EdgeProps) {
  const data = props.data as BeltEdgeData;

  let path: string;
  let labelX: number;
  let labelY: number;
  if (data.geom) {
    path = data.geom.path;
    labelX = data.geom.labelX;
    labelY = data.geom.labelY;
  } else {
    [path, labelX, labelY] = getBezierPath({
      sourceX: props.sourceX,
      sourceY: props.sourceY,
      targetX: props.targetX,
      targetY: props.targetY,
      sourcePosition: props.sourcePosition,
      targetPosition: props.targetPosition,
    });
  }

  const level = flowLevel(data.saturation);
  const capacity = beltCapacity(data.edge.tier);
  const s = data.flowOverlay ? STROKE[level] : { width: 2, dash: undefined, color: "var(--steel-500)" };
  const isCrit = data.flowOverlay && level === "crit";

  return (
    <>
      <BaseEdge
        id={props.id}
        path={path}
        style={{
          stroke: props.selected ? "var(--signal-500)" : s.color,
          strokeWidth: s.width,
          strokeDasharray: s.dash,
          strokeLinejoin: "round",
          strokeLinecap: "round",
          fill: "none",
          opacity: data.dimmed ? 0.15 : 1,
        }}
      />
      <EdgeLabelRenderer>
        <div
          className={`belt-label mono ${isCrit ? "crit" : ""} ${data.projected ? "projected" : ""} ${
            data.settled ? "settle" : ""
          } ${props.selected ? "selected" : ""} ${data.dimmed ? "dimmed" : ""}`}
          style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          data-testid={`belt-label-${data.edge.id}`}
        >
          {isCrit && "⚠ "}
          {data.lift && <span className="belt-lift">⇅ </span>}
          {fmtRate(data.flow)}/{fmtRate(capacity)} · {fmtPercent(data.saturation)}
          <span className="belt-tier">MK.{data.edge.tier}</span>
        </div>
      </EdgeLabelRenderer>
    </>
  );
}
