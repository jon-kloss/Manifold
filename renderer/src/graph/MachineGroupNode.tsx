// Machine-group card (mock 4b anatomy): header (icon 22 + MACHINE ×n + clock
// chip), recipe row, footer IN n / OUT n / power. Status grammar on the card
// frame; selected = 2px orange border + corner cut.

import { useState } from "react";
import { Handle, Position } from "@xyflow/react";
import { useStore } from "../state/store";
import { fmtClock, fmtPower, fmtRate } from "../lib/format";
import { footprintFor, FOOTPRINT_MAX_PX, FOOTPRINT_SCALE } from "./footprints";
import { POWER_ITEM, type GameData, type MachineGroup } from "../state/types";
import ItemIcon, { ICONS } from "../lib/ItemIcon";

export interface GroupNodeData {
  group: MachineGroup;
  factoryId: string;
  showFloorBadge?: boolean;
  [key: string]: unknown;
}

/** Top-down outlines at one shared px-per-meter scale — relative machine size
 *  reads truthfully across cards (a giant's outline caps at FOOTPRINT_MAX_PX
 *  so the card survives real clearance data). Dims come from the catalog's
 *  clearance footprint when it carries one, else the community estimate.
 *  Capped at 12 outlines with a +n overflow. */
function FootprintStrip({
  gamedata,
  machine,
  count,
}: {
  gamedata: GameData;
  machine: string;
  count: number;
}) {
  // Failed-load latch KEYED BY MACHINE CLASS: a machine swap on this card
  // retries the new class's icon. React owns the <img> node — never detach it
  // (the old onError remove() left a dead node that ate later src swaps); the
  // ICONS manifest gate kills guaranteed-404 requests for unvendored machines.
  const [failedIcon, setFailedIcon] = useState<string | null>(null);
  const f = footprintFor(gamedata, machine);
  const scale = Math.min(FOOTPRINT_SCALE, FOOTPRINT_MAX_PX / Math.max(f.w, f.l));
  const w = Math.max(5, Math.round(f.w * scale));
  const l = Math.max(5, Math.round(f.l * scale));
  const shown = Math.min(count, 12);
  const source = f.derived ? "game clearance data" : "community estimate";
  return (
    <div className="fp-strip" title={`${f.w} × ${f.l} m each — top-down footprint (${source})`}>
      <div className="fp-outlines">
        {Array.from({ length: shown }, (_, i) => (
          <span key={i} className="fp-box" style={{ width: w, height: l }}>
            {/* the machine render sits on the first pad; the rest stay bare
                outlines so a ×12 bank reads as pads, not a sprite sheet */}
            {i === 0 && ICONS.has(machine) && failedIcon !== machine && (
              <img src={`/icons/${machine}.png`} alt="" draggable={false} onError={() => setFailedIcon(machine)} />
            )}
          </span>
        ))}
        {count > shown && <span className="fp-more mono">+{count - shown}</span>}
      </div>
      <span className="fp-dims mono">
        {f.w} × {f.l} m
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
  // ◇ delta on a ◆ baseline (SDD §3.1.1): render the planned effective values.
  const deltaCount = group.plannedDelta?.count ?? null;
  const deltaClock = group.plannedDelta?.clock ?? null;
  const clockPct = deltaClock ?? group.clock;
  const clockClass = clockPct < 1 ? "clock-under" : clockPct > 1 ? "clock-over" : "";
  const justSettled = settled.has(`/groups/${group.id}`);
  const numCls = `${isProjected || group.status === "planned" ? "projected" : ""} ${justSettled ? "settle" : ""}`;

  const outRate = recipe?.products?.[0] ? dg?.outRates[recipe.products[0][0]] ?? 0 : 0;

  return (
    <div className={`group-card frame-${group.status} ${selected ? "selected" : ""}`} data-testid={`group-${group.recipe}`}>
      <Handle type="target" position={Position.Left} className="belt-handle" />
      <header className="group-card-head">
        <ItemIcon item={group.machine} displayName={machine} size={20} />
        <span className="group-card-name">
          {machine.toUpperCase()} <span className={`mono ${numCls}`}>×{group.count}</span>
          {deltaCount !== null && <span className="mono projected"> ➜ ×{deltaCount}</span>}
        </span>
        <span
          className={`chip clock-chip ${clockClass}`}
          title={deltaClock !== null ? `Clock — built at ${fmtClock(group.clock)}` : "Clock"}
        >
          {clockPct < 1 ? "↓" : clockPct > 1 ? "↑" : ""}
          <span className={deltaClock !== null ? `projected ${numCls}` : numCls}>{fmtClock(clockPct)}</span>
        </span>
      </header>
      <div className="group-card-recipe">
        <ItemIcon item={recipe?.products?.[0]?.[0] ?? ""} size={20} />
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
      <FootprintStrip gamedata={gamedata} machine={group.machine} count={deltaCount ?? group.count} />
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
