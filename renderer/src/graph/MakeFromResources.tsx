// "MAKE FROM RESOURCES" modal: given the factory's assigned input ports, list
// the items fully makeable from them; pick one + a rate and the planner builds
// the whole chain — machines sized to demand, belts wired to the existing input
// ports, and a fresh OUT port for the target. Two dispatches (groups+out port,
// then the belts that reference their new ids); TIDY afterwards straightens it.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { itemLabel } from "../lib/format";
import ItemIcon from "../lib/ItemIcon";
import type { Command, EdgeEnd, Id } from "../state/types";
import { makeableItems, planChain, type ChainGroup } from "./makeChain";

export default function MakeFromResources({
  factoryId,
  onClose,
}: {
  factoryId: Id;
  onClose: () => void;
}) {
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const unlocked = useStore((s) => s.unlocked);
  const dispatch = useStore((s) => s.dispatch);
  const pushToast = useStore((s) => s.pushToast);
  const setSelection = useStore((s) => s.setSelection);

  const [target, setTarget] = useState<string | null>(null);
  const [rate, setRate] = useState(20);
  const [busy, setBusy] = useState(false);

  // input ports on this factory → the raws we can build from.
  const inPorts = useMemo(
    () => Object.values(plan.ports).filter((p) => p.factory === factoryId && p.direction === "in"),
    [plan.ports, factoryId],
  );
  const available = useMemo(() => new Set(inPorts.map((p) => p.item)), [inPorts]);
  const makeable = useMemo(
    () => makeableItems(gamedata, unlocked, available),
    [gamedata, unlocked, available],
  );

  const name = (item: string) => itemLabel(gamedata.items, item);

  const build = async () => {
    if (!target || busy) return;
    setBusy(true);
    try {
      const cp = planChain(gamedata, unlocked, available, target, rate);
      if (!cp) {
        pushToast(`Couldn't plan ${name(target)} from these resources.`, "error");
        return;
      }
      // first port carrying each raw (a factory may have several of the same).
      const portForItem = new Map<string, Id>();
      for (const p of inPorts) if (!portForItem.has(p.item)) portForItem.set(p.item, p.id);

      // column layout by topological depth, anchored right of the input ports.
      const baseX = Math.max(0, ...inPorts.map((p) => p.graphPos.x)) + 300;
      const maxDepth = Math.max(1, ...cp.groups.map((g) => g.depth));
      const byDepth = new Map<number, ChainGroup[]>();
      for (const g of cp.groups) byDepth.set(g.depth, [...(byDepth.get(g.depth) ?? []), g]);
      const posOf = new Map<string, { x: number; y: number }>();
      for (const [d, gs] of byDepth) {
        gs.forEach((g, i) => posOf.set(g.item, { x: baseX + (d - 1) * 300, y: 80 + i * 190 }));
      }

      const groupCmds: Command[] = cp.groups.map((g) => ({
        type: "add_group",
        factory: factoryId,
        machine: g.machine,
        recipe: g.recipe,
        count: g.count,
        clock: g.clock,
        graphPos: posOf.get(g.item)!,
        floor: 0,
      }));
      const outCmd: Command = {
        type: "add_port",
        factory: factoryId,
        direction: "out",
        item: target,
        rate: 0,
        rateCeiling: null,
        graphPos: { x: baseX + maxDepth * 300, y: 80 },
      };

      const ids = await dispatch([...groupCmds, outCmd]);
      if (!ids) return;
      const groupId = new Map<string, Id>();
      cp.groups.forEach((g, i) => groupId.set(g.item, ids[i]));
      const outPortId = ids[cp.groups.length];

      const edgeCmds: Command[] = cp.belts.map((b) => {
        const from: EdgeEnd = b.fromRaw
          ? { kind: "port", id: portForItem.get(b.fromItem)! }
          : { kind: "group", id: groupId.get(b.fromItem)! };
        const to: EdgeEnd =
          b.toItem === "OUT"
            ? { kind: "port", id: outPortId }
            : { kind: "group", id: groupId.get(b.toItem)! };
        return { type: "add_edge", factory: factoryId, from, to, item: b.item, tier: b.tier };
      });
      await dispatch(edgeCmds);
      await dispatch([{ type: "tidy_layout", factory: factoryId }]).catch(() => {});

      setSelection(null);
      pushToast(`Built ${name(target)} — ${cp.groups.length} machine group(s) wired to your inputs.`, "success");
      onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mfr-scrim" data-testid="make-from-resources" onClick={onClose}>
      <div className="mfr-modal" onClick={(e) => e.stopPropagation()}>
        <header className="mfr-head">
          <span className="mfr-stamp mono">MAKE FROM RESOURCES</span>
          <span className="mono mfr-sub">
            from {inPorts.length === 0 ? "—" : [...available].map(name).join(" · ")}
          </span>
          <button className="drawer-close" onClick={onClose} aria-label="Close">
            ×
          </button>
        </header>

        {inPorts.length === 0 ? (
          <div className="mfr-empty">
            Assign input resources first — add IN ports (+ IN PORT) for the raws this factory has, then
            pick what to make.
          </div>
        ) : makeable.length === 0 ? (
          <div className="mfr-empty">
            Nothing is fully makeable from these inputs alone. Add more raw resources (e.g. another ore)
            to unlock recipes.
          </div>
        ) : (
          <>
            <div className="mfr-grid" data-testid="mfr-grid">
              {makeable.map((item) => (
                <button
                  key={item}
                  className={`mfr-item ${target === item ? "selected" : ""}`}
                  onClick={() => setTarget(item)}
                  data-testid={`mfr-item-${item}`}
                >
                  <ItemIcon item={item} displayName={name(item)} size={28} />
                  <span className="mfr-item-name">{name(item)}</span>
                </button>
              ))}
            </div>
            <footer className="mfr-foot">
              <label className="mfr-rate">
                <span className="t-label">RATE</span>
                <input
                  type="number"
                  min={1}
                  className="mono"
                  value={rate}
                  onChange={(e) => setRate(Math.max(1, Number(e.target.value) || 1))}
                  data-testid="mfr-rate"
                />
                <span className="unit mono">/min</span>
              </label>
              <button
                className="btn btn-primary"
                disabled={!target || busy}
                onClick={() => void build()}
                data-testid="mfr-build"
              >
                {busy ? "BUILDING…" : target ? `BUILD ${name(target).toUpperCase()}` : "PICK AN ITEM"}
              </button>
            </footer>
          </>
        )}
      </div>
    </div>
  );
}
