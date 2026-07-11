// Belt junction card (splitter/merger/storage): compact square-ish card —
// square = infrastructure grammar (A2.3). Shows the buildable's real name from
// the game-data catalog, its port budget, and the item it carries.

import { Handle, Position } from "@xyflow/react";
import { useStore } from "../state/store";
import { JUNCTION_CAPS, type Junction } from "../state/types";

export interface JunctionNodeData {
  junction: Junction;
  factoryId: string;
  showFloorBadge?: boolean;
  [key: string]: unknown;
}

const KIND_GLYPH: Record<Junction["kind"], string> = {
  splitter: "⑂",
  smart_splitter: "⑂",
  programmable_splitter: "⑂",
  merger: "⑃",
  storage: "▤",
};

export default function JunctionNode({ data, selected }: { data: JunctionNodeData; selected?: boolean }) {
  const { junction } = data;
  const gamedata = useStore((s) => s.gamedata);
  const plan = useStore((s) => s.plan);

  const name = gamedata.buildables?.[junction.buildable]?.displayName ?? junction.kind.replace("_", " ");
  const [inCap, outCap] = JUNCTION_CAPS[junction.kind];
  const edges = Object.values(plan.edges).filter(
    (e) =>
      (e.from.kind === "junction" && e.from.id === junction.id) ||
      (e.to.kind === "junction" && e.to.id === junction.id),
  );
  const inUsed = edges.filter((e) => e.to.kind === "junction" && e.to.id === junction.id).length;
  const outUsed = edges.filter((e) => e.from.kind === "junction" && e.from.id === junction.id).length;
  const item = edges[0] ? gamedata.items[edges[0].item]?.displayName : null;

  return (
    <div
      className={`junction-card frame-${junction.status} ${selected ? "selected" : ""}`}
      data-testid={`junction-${junction.kind}-${junction.id}`}
      title={`${name} — in ${inUsed}/${inCap} · out ${outUsed}/${outCap}`}
    >
      <Handle type="target" position={Position.Left} className="belt-handle" />
      <div className="junction-glyph" aria-hidden>
        {KIND_GLYPH[junction.kind]}
      </div>
      <div className="junction-body">
        <span className="junction-name">{name.toUpperCase()}</span>
        <span className="mono junction-meta">
          {inUsed}/{inCap} IN · {outUsed}/{outCap} OUT
          {data.showFloorBadge && <span className="floor-badge-foot"> F{junction.floor}</span>}
        </span>
        {item && <span className="mono junction-item">{item.toUpperCase()}</span>}
      </div>
      <Handle type="source" position={Position.Right} className="belt-handle" />
    </div>
  );
}
