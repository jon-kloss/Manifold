// Right-click context menu for the factory graph. Two entry points converge
// here: a single machine (offer to send each of its products out of the
// factory) and a box-selection (bulk delete / set clock / move to floor /
// duplicate). Every action is one undo step.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { itemLabel } from "../lib/format";
import { POWER_ITEM, effClock, effCount, type Command, type Id } from "../state/types";
import { minBeltTier } from "./logistics";

export interface CtxTarget {
  x: number;
  y: number;
  nodeIds: Id[];
}

export default function GraphContextMenu({
  target,
  factoryId,
  onClose,
}: {
  target: CtxTarget;
  factoryId: Id;
  onClose: () => void;
}) {
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const derived = useStore((s) => s.derived);
  const dispatch = useStore((s) => s.dispatch);
  const pushToast = useStore((s) => s.pushToast);
  const setSelection = useStore((s) => s.setSelection);
  const [sub, setSub] = useState<null | "clock" | "floor">(null);
  const [clockPct, setClockPct] = useState("100");

  const name = (item: string) => itemLabel(gamedata.items, item);

  // Partition the targeted nodes by kind.
  const { groupIds, junctionIds, portIds } = useMemo(() => {
    const groupIds: Id[] = [];
    const junctionIds: Id[] = [];
    const portIds: Id[] = [];
    for (const id of target.nodeIds) {
      if (plan.groups[id]) groupIds.push(id);
      else if (plan.junctions[id]) junctionIds.push(id);
      else if (plan.ports[id]) portIds.push(id);
    }
    return { groupIds, junctionIds, portIds };
  }, [target.nodeIds, plan]);

  const total = groupIds.length + junctionIds.length + portIds.length;
  const floors = useMemo(() => {
    const set = new Set<number>([0]);
    for (const gid of plan.factories[factoryId]?.groups ?? []) {
      const f = plan.groups[gid]?.floor;
      if (f != null) set.add(f);
    }
    for (const j of Object.values(plan.junctions)) if (j.factory === factoryId) set.add(j.floor);
    return [...set].sort((a, b) => a - b);
  }, [plan, factoryId]);

  const soleGroup = groupIds.length === 1 && junctionIds.length === 0 && portIds.length === 0 ? groupIds[0] : null;

  /** Shippable surplus of `item` at group `gid`: the group's NAMEPLATE output
   *  (what it can make, not the demand-driven realized rate — an unconsumed
   *  product solves to 0 yet is exactly what you want to send out) minus what's
   *  already committed: existing export-port targets + internal consumption. So
   *  a product already fully exported reports ~0 and isn't re-offered. */
  const surplus = (gid: Id, item: string): number => {
    const df = derived.factories[factoryId];
    const g = plan.groups[gid]!;
    const recipe = gamedata.recipes[g.recipe];
    const perMachine = recipe && recipe.durationS > 0
      ? ((recipe.products.find(([i]) => i === item)?.[1] ?? 0) * 60) / recipe.durationS
      : 0;
    // Nameplate capacity, NOT the demand-driven clock: an idle output solves to
    // clock 0, but that's precisely what you want to send out, so floor the
    // clock at 100% (and honor an overclock above it).
    const potential = perMachine * effCount(g) * Math.max(effClock(g), 1);
    let committed = 0;
    for (const e of Object.values(plan.edges)) {
      if (e.factory !== factoryId || e.item !== item || e.from.kind !== "group" || e.from.id !== gid) continue;
      committed +=
        e.to.kind === "port"
          ? (plan.ports[e.to.id]?.rate ?? 0) // already exported at this target
          : (df?.edges[e.id]?.flow ?? 0); // consumed internally
    }
    return Math.max(0, potential - committed);
  };

  // Single machine → offer to send each product that still has surplus out of
  // the factory. Products already fully routed/exported are not re-offered
  // (a second send would create a duplicate, over-subscribed output port).
  const sendable = useMemo(() => {
    if (!soleGroup) return [];
    const recipe = gamedata.recipes[plan.groups[soleGroup]!.recipe];
    return (recipe?.products ?? [])
      .map(([item]) => item)
      .filter((item) => item !== POWER_ITEM && surplus(soleGroup, item) > 1e-6);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [soleGroup, gamedata.recipes, plan.groups, plan.edges, derived]);

  const sendOut = async (gid: Id, item: string) => {
    const g = plan.groups[gid]!;
    const rate = surplus(gid, item);
    const outCount = Object.values(plan.ports).filter((p) => p.factory === factoryId && p.direction === "out").length;
    const ids = await dispatch([
      {
        type: "add_port",
        factory: factoryId,
        direction: "out",
        item,
        rate,
        rateCeiling: null,
        graphPos: { x: g.graphPos.x + 340, y: 80 + outCount * 128 },
      },
    ]);
    const portId = ids?.[0];
    if (portId) {
      await dispatch([
        {
          type: "add_edge",
          factory: factoryId,
          from: { kind: "group", id: gid },
          to: { kind: "port", id: portId },
          item,
          tier: minBeltTier(Math.max(rate, 1)),
        },
      ]);
      setSelection({ kind: "port", id: portId });
    }
    pushToast(`${name(item)} now exits the factory as an output.`, "success");
    onClose();
  };

  const setClockAll = async () => {
    const clock = Math.min(2.5, Math.max(0.01, (Number(clockPct) || 100) / 100));
    await dispatch(groupIds.map((id) => ({ type: "set_group_clock", id, clock }) as Command));
    pushToast(`Set ${groupIds.length} machine group(s) to ${Math.round(clock * 100)}% clock.`, "success");
    onClose();
  };

  const moveToFloor = async (floor: number) => {
    await dispatch([
      ...groupIds.map((id) => ({ type: "set_group_floor", id, floor }) as Command),
      ...junctionIds.map((id) => ({ type: "set_junction_floor", id, floor }) as Command),
    ]);
    pushToast(`Moved ${groupIds.length + junctionIds.length} to floor ${floor}.`, "success");
    onClose();
  };

  const duplicate = async () => {
    const addCmds: Command[] = groupIds.map((gid) => {
      const g = plan.groups[gid]!;
      return {
        type: "add_group",
        factory: factoryId,
        machine: g.machine,
        recipe: g.recipe,
        count: g.count,
        clock: g.clock,
        graphPos: { x: g.graphPos.x + 56, y: g.graphPos.y + 56 },
        floor: g.floor,
      };
    });
    const ids = await dispatch(addCmds);
    if (!ids) return;
    const idMap = new Map(groupIds.map((gid, i) => [gid, ids[i]]));
    // Re-create the belts that live ENTIRELY inside the copied set.
    const edgeCmds: Command[] = Object.values(plan.edges)
      .filter(
        (e) =>
          e.factory === factoryId &&
          e.from.kind === "group" &&
          e.to.kind === "group" &&
          idMap.has(e.from.id) &&
          idMap.has(e.to.id),
      )
      .map((e) => ({
        type: "add_edge",
        factory: factoryId,
        from: { kind: "group", id: idMap.get(e.from.id)! },
        to: { kind: "group", id: idMap.get(e.to.id)! },
        item: e.item,
        tier: e.tier,
      }));
    if (edgeCmds.length) await dispatch(edgeCmds);
    pushToast(`Duplicated ${groupIds.length} machine group(s).`, "success");
    onClose();
  };

  const del = async () => {
    await dispatch([
      ...groupIds.map((id) => ({ type: "delete_group", id }) as Command),
      ...junctionIds.map((id) => ({ type: "delete_junction", id }) as Command),
      ...portIds.map((id) => ({ type: "delete_port", id }) as Command),
    ]);
    setSelection(null);
    pushToast(`Deleted ${total} item(s). ⌘Z to undo.`, "success");
    onClose();
  };

  if (total === 0) return null;
  const style = { left: Math.min(target.x, window.innerWidth - 250), top: Math.min(target.y, window.innerHeight - 320) };

  return (
    <>
      <div className="ctx-backdrop" onClick={onClose} onContextMenu={(e) => { e.preventDefault(); onClose(); }} />
      <div className="ctx-menu" style={style} data-testid="graph-ctx-menu" onContextMenu={(e) => e.preventDefault()}>
        <div className="ctx-head mono">
          {soleGroup
            ? (gamedata.recipes[plan.groups[soleGroup]!.recipe]?.displayName ?? "MACHINE").toUpperCase()
            : `${total} SELECTED`}
        </div>

        {soleGroup &&
          sendable.map((item) => (
            <button key={item} className="ctx-item" data-testid={`ctx-send-${item}`} onClick={() => void sendOut(soleGroup, item)}>
              Send <b>{name(item)}</b> out of factory →
            </button>
          ))}

        {groupIds.length > 0 && (
          <>
            {sub === "clock" ? (
              <div className="ctx-sub">
                <span className="ctx-sub-label">Clock %</span>
                <input
                  autoFocus
                  type="number"
                  min={1}
                  max={250}
                  className="mono"
                  value={clockPct}
                  onChange={(e) => setClockPct(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void setClockAll();
                    if (e.key === "Escape") setSub(null);
                  }}
                  data-testid="ctx-clock-input"
                />
                <button className="ctx-apply" onClick={() => void setClockAll()} data-testid="ctx-clock-apply">
                  SET
                </button>
              </div>
            ) : (
              <button className="ctx-item" onClick={() => setSub("clock")} data-testid="ctx-set-clock">
                Set clock for all…
              </button>
            )}
          </>
        )}

        {groupIds.length + junctionIds.length > 0 && (
          <>
            {sub === "floor" ? (
              <div className="ctx-floors">
                <span className="ctx-sub-label">Move to floor</span>
                <div className="ctx-floor-row">
                  {floors.map((f) => (
                    <button key={f} className="ctx-floor" onClick={() => void moveToFloor(f)} data-testid={`ctx-floor-${f}`}>
                      {f}
                    </button>
                  ))}
                  <button className="ctx-floor new" onClick={() => void moveToFloor((floors.at(-1) ?? 0) + 1)}>
                    +{(floors.at(-1) ?? 0) + 1}
                  </button>
                </div>
              </div>
            ) : (
              <button className="ctx-item" onClick={() => setSub("floor")} data-testid="ctx-move-floor">
                Move to floor…
              </button>
            )}
          </>
        )}

        {groupIds.length > 0 && (
          <button className="ctx-item" onClick={() => void duplicate()} data-testid="ctx-duplicate">
            Duplicate {groupIds.length > 1 ? `${groupIds.length} machines` : "machine"}
          </button>
        )}

        <button className="ctx-item ctx-danger" onClick={() => void del()} data-testid="ctx-delete">
          Delete {total > 1 ? `${total} items` : "item"}
        </button>
      </div>
    </>
  );
}
