// Belt edge with the flow encoding (mock 1e): three channels agree — color +
// thickness + pattern — plus the always-present mono label chip `n/cap · %`.
// Planned numbers italic; CRIT chip inverts with ⚠.

import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { flowLevel } from "../lib/format";
import { fmtRate, fmtPercent } from "../lib/format";
import { beltCapacity, type BeltEdge } from "../state/types";

export interface BeltEdgeData {
  edge: BeltEdge;
  flow: number;
  saturation: number;
  projected: boolean;
  flowOverlay: boolean;
  settled: boolean;
  [key: string]: unknown;
}

const STROKE = {
  ok: { width: 2, dash: undefined, color: "var(--flow-ok)" },
  warn: { width: 4, dash: "10 5", color: "var(--flow-warn)" },
  crit: { width: 6, dash: "6 4", color: "var(--flow-crit)" },
} as const;

export default function BeltEdgeView(props: EdgeProps) {
  const data = props.data as BeltEdgeData;
  const [path, labelX, labelY] = getBezierPath({
    sourceX: props.sourceX,
    sourceY: props.sourceY,
    targetX: props.targetX,
    targetY: props.targetY,
    sourcePosition: props.sourcePosition,
    targetPosition: props.targetPosition,
  });

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
        }}
      />
      <EdgeLabelRenderer>
        <div
          className={`belt-label mono ${isCrit ? "crit" : ""} ${data.projected ? "projected" : ""} ${
            data.settled ? "settle" : ""
          } ${props.selected ? "selected" : ""}`}
          style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          data-testid={`belt-label-${data.edge.id}`}
        >
          {isCrit && "⚠ "}
          {fmtRate(data.flow)}/{fmtRate(capacity)} · {fmtPercent(data.saturation)}
          <span className="belt-tier">MK.{data.edge.tier}</span>
        </div>
      </EdgeLabelRenderer>
    </>
  );
}
