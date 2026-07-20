// Belt edge with the flow encoding (efficiency grammar): color + thickness +
// pattern agree, plus the always-present mono label chip `n/cap · %`. Paths
// are belt-style orthogonal runs from edgeLayout (consistent anchors, rounded
// corners, hop arcs over crossed belts). Cross-floor belts are lifts (⇅).

import type { CSSProperties } from "react";
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { flowBand, flowSpeed, type FlowBand } from "../lib/format";
import { fmtRate, fmtPercent } from "../lib/format";
import { beltCapacity, pipeCapacity, type BeltEdge } from "../state/types";
import type { EdgeGeom } from "./edgeLayout";

export interface LiftPortal {
  x: number;
  y: number;
  dir: "up" | "down";
  otherFloor: number;
}

export interface BeltEdgeData {
  edge: BeltEdge;
  /** true when this edge carries a fluid — rendered as a PIPE (blue, "PIPE
   *  Mk.n", 300/600 m³/min capacity) instead of a belt. */
  fluid: boolean;
  flow: number;
  saturation: number;
  /** solver-named evidence this belt caps demanded throughput (GraphView
   *  derives it from the factory's shortfall/ceiling bindings) */
  bottleneck: boolean;
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
  /** MANIFOLD motion 7l/7m: freshly-created edge extends from its neighbor
   *  (scaleX 0→1, 200ms) after this delay — undefined = no mount animation. */
  mountDelayMs?: number;
  [key: string]: unknown;
}

// Efficiency grammar (DECISIONS): green = good (>50% utilized, incl. a FULL
// belt meeting demand — optimal), amber dashed = under-used (flowing ≤50%),
// red heavy = bottleneck (solver-named: this belt caps demanded throughput).
const STROKE: Record<FlowBand, { width: number; dash?: string; color: string }> = {
  // Idle = wired but carrying nothing: a dim, sparsely-dotted neutral line so
  // it never reads as a healthy (green) belt. Distinct from under-used (amber).
  idle: { width: 1.5, dash: "2 6", color: "var(--steel-500)" },
  good: { width: 2, dash: undefined, color: "var(--flow-ok)" },
  under: { width: 2, dash: "10 5", color: "var(--flow-warn)" },
  bottleneck: { width: 6, dash: "6 4", color: "var(--flow-crit)" },
};

const BAND_NOTE: Record<FlowBand, string> = {
  idle: " · IDLE — connected but carrying nothing (its feed may be fully exported)",
  good: "",
  under: " · UNDER-USED — over-built or starved upstream",
  bottleneck: " · BOTTLENECK — this belt caps demanded throughput",
};

// Dash-period duration comes from the shared flowSpeed (lib/format.ts):
// MOTION = FLOW (gate: flow > 0); speed = utilization. The keyframes travel
// a fixed 18px period, so speed scales inversely with the duration.

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

  const band = flowBand(data.saturation, data.flow, data.bottleneck);
  // Clamp a stale belt tier (a legacy fluid edge saved before pipes) into the
  // medium's real range so the label and capacity agree — pipes reach Mk.2.
  const tier = data.fluid ? Math.max(1, Math.min(2, data.edge.tier)) : data.edge.tier;
  const capacity = data.fluid ? pipeCapacity(tier) : beltCapacity(tier);
  // Pipes carry the same flow-health grammar (green/amber/red), but their
  // neutral (no-overlay) state tints blue so a fluid run reads as a PIPE at a
  // glance, never a belt.
  const neutral = data.fluid ? "var(--bp-400)" : "var(--steel-500)";
  const s = data.flowOverlay ? STROKE[band] : { width: 2, dash: undefined, color: neutral };
  // Tier label reads "PIPE Mk.n" for fluids, "MK.n" for belts.
  const tierLabel = data.fluid ? `PIPE Mk.${tier}` : `MK.${tier}`;
  const isBottleneck = data.flowOverlay && band === "bottleneck";
  // MOTION = FLOW (gate: flow > 0); speed = utilization: only edges with
  // derived flow > 0 animate; idle belts stay static — independent of the
  // band (an under-used belt still trickles).
  // Motion rides a separate neutral-ink overlay path so the base line keeps
  // its status color + weight (color stays status-only).
  const flowing = data.flowOverlay && data.flow > 0;
  // Very short belts can't carry the full chip — compact to the saturation %
  // (the load signal), full detail on hover and in the inspector. BOTTLENECK
  // belts always show the full chip: the alarm outranks tidiness.
  const compact = !isBottleneck && !props.selected && (data.geom?.pathLen ?? Infinity) < 150;
  const liftTag = data.lift ? `⇅ F${data.srcFloor}→F${data.dstFloor}` : "";
  const fullText = `${liftTag ? liftTag + " · " : ""}${fmtRate(data.flow)}/${fmtRate(capacity)} · ${fmtPercent(data.saturation)} ${tierLabel}${data.flowOverlay ? BAND_NOTE[band] : ""}`;

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

  // Motion 7l/7m: a just-created belt extends from its neighbor — scaleX 0→1
  // over 200ms after the choreographed delay (after its card in 7l; just
  // before its card in a 7m chain). Inline so the delay can vary per edge;
  // the keyframes live in graph.css and reduced-motion is enforced upstream
  // (GraphView never sets mountDelayMs under prefers-reduced-motion).
  // The growth origin is the SOURCE end (the neighbor it extends from):
  // geometrically that's the left edge of the bounding box except when the
  // path runs right → left (a feedback belt) — then it's the right edge.
  const srcIsRight =
    data.geom && data.geom.points.length >= 2 && data.geom.points[0].x > data.geom.points[data.geom.points.length - 1].x;
  const mountAnim: CSSProperties | undefined =
    data.mountDelayMs !== undefined
      ? {
          transformBox: "fill-box",
          transformOrigin: srcIsRight ? "right center" : "left center",
          animation: `mfd-edge-extend 200ms var(--ease) ${data.mountDelayMs}ms backwards`,
        }
      : undefined;

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
          ...mountAnim,
        }}
      />
      {flowing && data.mountDelayMs === undefined && (
        // moving-highlight overlay: dashes travel source→target (negative
        // dashoffset animation along the RF source→target path direction).
        // Suppressed during the mount extend — its dash animation would fight
        // the extend transform; it appears when the transient mount clears.
        <path
          d={path}
          className="edge-flowing"
          data-testid={`edge-flowing-${data.edge.id}`}
          style={
            {
              "--flow-speed": flowSpeed(data.saturation),
              opacity: data.dimmed ? 0.15 : undefined,
            } as CSSProperties
          }
        />
      )}
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
          className={`belt-label mono ${data.fluid ? "pipe" : ""} ${data.flowOverlay ? band : ""} ${compact ? "compact" : ""} ${
            data.projected ? "projected" : ""
          } ${data.settled ? "settle" : ""} ${props.selected ? "selected" : ""} ${data.dimmed ? "dimmed" : ""}`}
          style={{
            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
            ...(data.mountDelayMs !== undefined
              ? { animation: `mfd-fade-in 150ms var(--ease) ${data.mountDelayMs + 200}ms backwards` }
              : {}),
          }}
          title={fullText}
          data-testid={`belt-label-${data.edge.id}`}
        >
          {isBottleneck && "⚠ "}
          {data.lift && <span className="belt-lift">{compact ? "⇅ " : `${liftTag} · `}</span>}
          {compact ? (
            fmtPercent(data.saturation)
          ) : (
            <>
              {fmtRate(data.flow)}/{fmtRate(capacity)} · {fmtPercent(data.saturation)}
              <span className="belt-tier">{tierLabel}</span>
            </>
          )}
        </div>
      </EdgeLabelRenderer>
    </>
  );
}
