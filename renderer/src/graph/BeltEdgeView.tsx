// Belt edge with the flow encoding (mock 1e): color + thickness + pattern agree,
// plus the always-present mono label chip `n/cap · %`. Paths are belt-style
// orthogonal runs from edgeLayout (consistent anchors, rounded corners, hop
// arcs over crossed belts). Cross-floor belts are lifts (⇅).

import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { flowLevel } from "../lib/format";
import { fmtRate, fmtPercent } from "../lib/format";
import { beltCapacity, type BeltEdge } from "../state/types";
import type { EdgeGeom } from "./edgeLayout";

export interface LiftPortal {
  x: number;
  y: number;
  dir: "up" | "down";
  otherFloor: number;
}

export interface BeltEdgeData {
  edge: BeltEdge;
  flow: number;
  saturation: number;
  projected: boolean;
  flowOverlay: boolean;
  settled: boolean;
  geom: EdgeGeom | null;
  lift: boolean;
  srcFloor: number;
  dstFloor: number;
  /** floor-filtered view: this belt leaves the visible floor here */
  portal: LiftPortal | null;
  onJumpFloor?: (floor: number, edgeId: string) => void;
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
  // Very short belts can't carry the full chip — compact to the saturation %
  // (the load signal), full detail on hover and in the inspector. CRIT belts
  // always show the full chip: the alarm outranks tidiness.
  const compact = !isCrit && !props.selected && (data.geom?.pathLen ?? Infinity) < 150;
  const liftTag = data.lift ? `⇅ F${data.srcFloor}→F${data.dstFloor}` : "";
  const fullText = `${liftTag ? liftTag + " · " : ""}${fmtRate(data.flow)}/${fmtRate(capacity)} · ${fmtPercent(data.saturation)} MK.${data.edge.tier}`;

  // Floor-filtered view: the belt runs to a lift portal at the card edge.
  // Clicking the portal jumps to the connected floor with this belt selected.
  if (data.portal) {
    const portal = data.portal;
    return (
      <>
        <BaseEdge
          id={props.id}
          path={path}
          style={{
            stroke: props.selected ? "var(--signal-500)" : s.color,
            strokeWidth: 2,
            strokeDasharray: "3 4",
            fill: "none",
          }}
        />
        <EdgeLabelRenderer>
          <button
            className={`lift-portal ${props.selected ? "selected" : ""}`}
            style={{ transform: `translate(-50%, -50%) translate(${portal.x}px, ${portal.y}px)` }}
            title={`${fullText} — jump to floor ${portal.otherFloor}`}
            onClick={() => data.onJumpFloor?.(portal.otherFloor, data.edge.id)}
            data-testid={`lift-portal-${data.edge.id}`}
          >
            <span className="lift-portal-pin" data-dir={portal.dir}>
              {portal.dir === "up" ? "⤒" : "⤓"}
            </span>
            <span className="mono lift-portal-chip">
              F{portal.otherFloor} · {fmtPercent(data.saturation)}
            </span>
          </button>
        </EdgeLabelRenderer>
      </>
    );
  }

  const pads =
    data.lift && data.geom && data.geom.points.length >= 2
      ? [data.geom.points[0], data.geom.points[data.geom.points.length - 1]]
      : [];

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
      {pads.map((pt, i) => (
        // lift pads: square nubs where a cross-floor belt meets its cards
        <rect
          key={i}
          className="lift-pad"
          x={pt.x - 5}
          y={pt.y - 5}
          width={10}
          height={10}
          transform={`rotate(45 ${pt.x} ${pt.y})`}
          style={{ opacity: data.dimmed ? 0.15 : 1 }}
        />
      ))}
      <EdgeLabelRenderer>
        <div
          className={`belt-label mono ${isCrit ? "crit" : ""} ${compact ? "compact" : ""} ${
            data.projected ? "projected" : ""
          } ${data.settled ? "settle" : ""} ${props.selected ? "selected" : ""} ${data.dimmed ? "dimmed" : ""}`}
          style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
          title={fullText}
          data-testid={`belt-label-${data.edge.id}`}
        >
          {isCrit && "⚠ "}
          {data.lift && <span className="belt-lift">{compact ? "⇅ " : `${liftTag} · `}</span>}
          {compact ? (
            fmtPercent(data.saturation)
          ) : (
            <>
              {fmtRate(data.flow)}/{fmtRate(capacity)} · {fmtPercent(data.saturation)}
              <span className="belt-tier">MK.{data.edge.tier}</span>
            </>
          )}
        </div>
      </EdgeLabelRenderer>
    </>
  );
}
