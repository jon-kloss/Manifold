// Machine-group card (mock 4b anatomy): header (icon 22 + MACHINE ×n + clock
// chip), recipe row, footer IN n / OUT n / power. Status grammar on the card
// frame; selected = 2px orange border + corner cut.

import { Handle, Position } from "@xyflow/react";
import { useStore } from "../state/store";
import { fmtClock, fmtPower, fmtRate } from "../lib/format";
import { footprintOf, FOOTPRINT_SCALE } from "./footprints";
import { POWER_ITEM, type MachineGroup } from "../state/types";

export interface GroupNodeData {
  group: MachineGroup;
  factoryId: string;
  showFloorBadge?: boolean;
  [key: string]: unknown;
}

/** Top-down outlines at one shared px-per-meter scale — relative machine size
 *  reads truthfully across cards. Capped at 12 outlines with a +n overflow. */
function FootprintStrip({ machine, count }: { machine: string; count: number }) {
  const f = footprintOf(machine);
  const w = Math.max(5, Math.round(f.w * FOOTPRINT_SCALE));
  const l = Math.max(5, Math.round(f.l * FOOTPRINT_SCALE));
  const shown = Math.min(count, 12);
  return (
    <div className="fp-strip" title={`${f.w}×${f.l} m each — top-down footprint`}>
      <div className="fp-outlines">
        {Array.from({ length: shown }, (_, i) => (
          <span key={i} className="fp-box" style={{ width: w, height: l }} />
        ))}
        {count > shown && <span className="fp-more mono">+{count - shown}</span>}
      </div>
      <span className="fp-dims mono">
        {f.w}×{f.l}M
      </span>
    </div>
  );
}

export default function MachineGroupNode({ data, selected }: { data: GroupNodeData; selected?: boolean }) {
  const { group } = data;
  const gamedata = useStore((s) => s.gamedata);
  const derived = useStore((s) => s.derived);
  const projected = useStore((s) => s.projected);
  const settled = useStore((s) => s.settled);

  const df = projected && projected.factoryId === group.factory ? projected.result : derived.factories[group.factory];
  const dg = df?.groups[group.id];
  const isProjected = !!projected && projected.factoryId === group.factory;

  const machine = gamedata.machines[group.machine]?.displayName ?? group.machine;
  const recipe = gamedata.recipes[group.recipe];
  const clockPct = group.clock;
  const clockClass = clockPct < 1 ? "clock-under" : clockPct > 1 ? "clock-over" : "";
  const justSettled = settled.has(`/groups/${group.id}`);
  const numCls = `${isProjected || group.status === "planned" ? "projected" : ""} ${justSettled ? "settle" : ""}`;

  const outRate = recipe?.products?.[0] ? dg?.outRates[recipe.products[0][0]] ?? 0 : 0;

  return (
    <div className={`group-card frame-${group.status} ${selected ? "selected" : ""}`} data-testid={`group-${group.recipe}`}>
      <Handle type="target" position={Position.Left} className="belt-handle" />
      <header className="group-card-head">
        <div className="icon-ph s20" />
        <span className="group-card-name">
          {machine.toUpperCase()} <span className={`mono ${numCls}`}>×{group.count}</span>
        </span>
        <span className={`chip clock-chip ${clockClass}`} title="Clock">
          {clockPct < 1 ? "↓" : clockPct > 1 ? "↑" : ""}
          <span className={numCls}>{fmtClock(group.clock)}</span>
        </span>
      </header>
      <div className="group-card-recipe">
        <div className="icon-ph s20" />
        <span>{recipe?.displayName ?? group.recipe}</span>
        <span className={`t-data-12 ${numCls}`} style={{ marginLeft: "auto" }}>
          {recipe?.products?.[0]?.[0] === POWER_ITEM ? (
            fmtPower(outRate) // generators: 1 pseudo-item/min ≡ 1 MW
          ) : (
            <>
              {fmtRate(outRate)}
              <span className="unit">/min</span>
            </>
          )}
        </span>
      </div>
      <FootprintStrip machine={group.machine} count={group.count} />
      <footer className="group-card-foot mono">
        <span>IN {recipe?.ingredients.length ?? 0}</span>
        <span>OUT {recipe?.products.length ?? 0}</span>
        {data.showFloorBadge && <span className="floor-badge-foot">F{group.floor}</span>}
        <span className={numCls}>{fmtPower(dg?.powerMw ?? 0)}</span>
      </footer>
      <Handle type="source" position={Position.Right} className="belt-handle" />
    </div>
  );
}
