// Belt junction node (splitter/merger/storage): a compact SQUARE, not a machine
// card — a junction produces nothing, it just routes belts (infrastructure
// grammar, A2.3). Shows the real buildable sprite from the vendored icon set
// (falling back to a routing glyph); name / port budget / item live in the
// hover tooltip.

import { useState } from "react";
import { Handle, Position } from "@xyflow/react";
import { useStore } from "../state/store";
import { itemLabel } from "../lib/format";
import { ICONS } from "../lib/ItemIcon";
import { JUNCTION_CAPS, isPipeJunction, type Junction } from "../state/types";

export interface JunctionNodeData {
  junction: Junction;
  factoryId: string;
  showFloorBadge?: boolean;
  /** MANIFOLD motion (§5): transient mount grammar ("mount-pop"). */
  motionCls?: string;
  [key: string]: unknown;
}

const KIND_GLYPH: Record<Junction["kind"], string> = {
  splitter: "⑂",
  smart_splitter: "⑂",
  programmable_splitter: "⑂",
  merger: "⑃",
  storage: "▤",
  pipe_junction: "╬",
};

export default function JunctionNode({ data, selected }: { data: JunctionNodeData; selected?: boolean }) {
  const { junction } = data;
  const gamedata = useStore((s) => s.gamedata);
  const plan = useStore((s) => s.plan);
  // Failed-load latch keyed by buildable class: a missing sprite falls back to
  // the routing glyph rather than a broken-image icon.
  const [failedIcon, setFailedIcon] = useState<string | null>(null);

  const name = gamedata.buildables?.[junction.buildable]?.displayName ?? junction.kind.replace("_", " ");
  const [inCap, outCap] = JUNCTION_CAPS[junction.kind];
  const edges = Object.values(plan.edges).filter(
    (e) =>
      (e.from.kind === "junction" && e.from.id === junction.id) ||
      (e.to.kind === "junction" && e.to.id === junction.id),
  );
  const inUsed = edges.filter((e) => e.to.kind === "junction" && e.to.id === junction.id).length;
  const outUsed = edges.filter((e) => e.from.kind === "junction" && e.from.id === junction.id).length;
  const item = edges[0] ? itemLabel(gamedata.items, edges[0].item) : null;
  const hasSprite = ICONS.has(junction.buildable) && failedIcon !== junction.buildable;
  const isMerger = junction.kind === "merger";
  const isStorage = junction.kind === "storage";
  const isPipe = isPipeJunction(junction.kind);

  return (
    // A junction just routes belts — it produces nothing, so it reads as a
    // square (infrastructure grammar), NOT a machine card. The real buildable
    // sprite sits inside; name / port budget / item live in the hover tooltip.
    // Handle faces mirror the game building: a splitter feeds in on the left and
    // splits out the other three sides; a merger is the exact opposite. Belts
    // are drawn from edgeLayout anchors, so these handles are the connection
    // affordance + side nubs that agree with where the belts run.
    <div
      className={`junction-card frame-${junction.status} ${isPipe ? "pipe" : ""} ${selected ? "selected" : ""} ${data.motionCls ?? ""}`}
      data-testid={`junction-${junction.kind}-${junction.id}`}
      // A pipe cross's real limit is a TOTAL of 4 ports in any in/out mix, so it
      // reads "N/4 ports" — showing "in 2/4 · out 2/4" would imply 4 more are
      // free when the next connection of either kind is refused. Belt junctions
      // keep the per-direction budget (their in/out caps genuinely differ).
      title={
        isPipe
          ? `${name} — ${inUsed + outUsed}/4 ports${item ? ` · ${item.toUpperCase()}` : ""}`
          : `${name} — in ${inUsed}/${inCap} · out ${outUsed}/${outCap}${item ? ` · ${item.toUpperCase()}` : ""}`
      }
    >
      {isStorage ? (
        <>
          <Handle type="target" position={Position.Left} className="belt-handle" />
          <Handle type="source" position={Position.Right} className="belt-handle" />
        </>
      ) : isPipe ? (
        // The cross both merges and splits: two input faces (left, top) and two
        // output faces (right, bottom). Server enforces the 4-port total.
        <>
          <Handle id="in-l" type="target" position={Position.Left} className="belt-handle" />
          <Handle id="in-t" type="target" position={Position.Top} className="belt-handle" />
          <Handle id="out-r" type="source" position={Position.Right} className="belt-handle" />
          <Handle id="out-b" type="source" position={Position.Bottom} className="belt-handle" />
        </>
      ) : isMerger ? (
        <>
          <Handle id="in-t" type="target" position={Position.Top} className="belt-handle" />
          <Handle id="in-l" type="target" position={Position.Left} className="belt-handle" />
          <Handle id="in-b" type="target" position={Position.Bottom} className="belt-handle" />
          <Handle id="out" type="source" position={Position.Right} className="belt-handle" />
        </>
      ) : (
        <>
          <Handle id="in" type="target" position={Position.Left} className="belt-handle" />
          <Handle id="out-t" type="source" position={Position.Top} className="belt-handle" />
          <Handle id="out-r" type="source" position={Position.Right} className="belt-handle" />
          <Handle id="out-b" type="source" position={Position.Bottom} className="belt-handle" />
        </>
      )}
      {hasSprite ? (
        <img
          className="junction-sprite"
          src={`/icons/${junction.buildable}.png`}
          alt=""
          draggable={false}
          onError={() => setFailedIcon(junction.buildable)}
        />
      ) : (
        <span className="junction-glyph" aria-hidden>
          {KIND_GLYPH[junction.kind]}
        </span>
      )}
      {data.showFloorBadge && <span className="junction-floor mono">F{junction.floor}</span>}
    </div>
  );
}
