// "MAKE FROM RESOURCES" modal: given the factory's assigned input ports, list
// the items fully makeable from them; pick one + a rate and the planner builds
// the whole chain — machines sized to demand, belts wired to the existing input
// ports, and a fresh OUT port for the target (targeted at the requested rate so
// the chain actually runs). Guards:
//   • node capacity — the claimed nodes must be able to feed the raw demand
//     (checked against extraction HEADROOM: ceiling − what's already drawn), or
//     the build is blocked with a warning + a "free up the node" action;
//   • smart reuse — if an intermediate is already produced in this factory
//     (e.g. rods when you ask for screws), reuse & scale that group instead of
//     duplicating it (opt-in checkbox, on by default).

import { useEffect, useMemo, useState } from "react";
import { useStore } from "../state/store";
import { fmtRate, itemLabel } from "../lib/format";
import ItemIcon from "../lib/ItemIcon";
import { POWER_ITEM, effClock, effCount, type Command, type EdgeEnd, type Id } from "../state/types";
import { makeableItems, planChain, type ChainGroup, type ChainPlan } from "./makeChain";

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
  const derived = useStore((s) => s.derived);
  const dispatch = useStore((s) => s.dispatch);
  const pushToast = useStore((s) => s.pushToast);
  const setSelection = useStore((s) => s.setSelection);

  const [target, setTarget] = useState<string | null>(null);
  const [rate, setRate] = useState(20);
  const [busy, setBusy] = useState(false);
  const [reuseOn, setReuseOn] = useState(true);
  const [confirmFree, setConfirmFree] = useState(false);

  const name = (item: string) => itemLabel(gamedata.items, item);

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

  // Existing groups in THIS factory, indexed by the item they produce — the
  // reuse candidates. (First producer wins if several make the same item.)
  const factoryGroups = useMemo(
    () => Object.values(plan.groups).filter((g) => g.factory === factoryId),
    [plan.groups, factoryId],
  );
  const existingProducers = useMemo(() => {
    const m = new Map<string, { id: Id; per: number; count: number; clock: number }>();
    for (const g of factoryGroups) {
      const r = gamedata.recipes[g.recipe];
      if (!r) continue;
      for (const [item, qty] of r.products) {
        if (item === POWER_ITEM || m.has(item)) continue;
        const per = r.durationS > 0 ? (qty * 60) / r.durationS : 0;
        if (per > 0) m.set(item, { id: g.id, per, count: effCount(g), clock: effClock(g) });
      }
    }
    return m;
  }, [factoryGroups, gamedata.recipes]);

  // Fresh chain (no reuse): the source of truth for capacity (full raw demand)
  // and for spotting reuse candidates. Reuse only changes wiring, not raw totals.
  const freshCp = useMemo<ChainPlan | null>(
    () => (target ? planChain(gamedata, unlocked, available, target, rate) : null),
    [gamedata, unlocked, available, target, rate],
  );
  const reusable = useMemo(
    () =>
      freshCp
        ? [...new Set(freshCp.groups.map((g) => g.item))].filter(
            (item) => item !== target && existingProducers.has(item),
          )
        : [],
    [freshCp, existingProducers, target],
  );
  const reuseItems = useMemo(
    () => (reuseOn ? new Set(reusable) : new Set<string>()),
    [reuseOn, reusable],
  );

  // Build chain: reused intermediates become leaves (wired to their existing
  // group), so they aren't rebuilt.
  const effectiveAvailable = useMemo(
    () => new Set([...available, ...reuseItems]),
    [available, reuseItems],
  );
  const buildCp = useMemo<ChainPlan | null>(
    () => (target ? planChain(gamedata, unlocked, effectiveAvailable, target, rate) : null),
    [gamedata, unlocked, effectiveAvailable, target, rate],
  );

  // Extraction HEADROOM per raw = Σ (ceiling − already-drawn) across input ports.
  // Subtracting current draw makes the check correct on a factory that already
  // produces things, and keeps it consistent whether we reuse or build fresh.
  // A null ceiling = supply assumed = unlimited.
  const headroom = useMemo(() => {
    const df = derived.factories[factoryId];
    const h = new Map<string, number>();
    for (const p of inPorts) {
      if (p.rateCeiling == null) {
        h.set(p.item, Infinity);
        continue;
      }
      if (h.get(p.item) === Infinity) continue;
      const used = df?.ports[p.id] ?? 0;
      h.set(p.item, (h.get(p.item) ?? 0) + Math.max(0, p.rateCeiling - used));
    }
    return h;
  }, [inPorts, derived.factories, factoryId]);

  // Full raw demand of the target (from the fresh plan — the true extraction
  // this build adds regardless of reuse).
  const rawDemand = useMemo(() => {
    const d = new Map<string, number>();
    if (freshCp) for (const b of freshCp.belts) if (b.fromRaw) d.set(b.fromItem, (d.get(b.fromItem) ?? 0) + b.rate);
    return d;
  }, [freshCp]);

  const shortfalls = useMemo(() => {
    const out: { item: string; need: number; have: number }[] = [];
    for (const [raw, need] of rawDemand) {
      const have = headroom.get(raw) ?? 0;
      if (need > have + 1e-6) out.push({ item: raw, need, have });
    }
    return out;
  }, [rawDemand, headroom]);

  const feasibleRate = useMemo(() => {
    if (shortfalls.length === 0) return rate;
    let ratio = Infinity;
    for (const [raw, need] of rawDemand) if (need > 0) ratio = Math.min(ratio, (headroom.get(raw) ?? 0) / need);
    return Math.floor(rate * ratio);
  }, [shortfalls, rawDemand, headroom, rate]);

  // Free-up: existing groups in THIS factory that consume a short raw. Removing
  // them returns that extraction to the pool so this build can use it.
  const freeable = useMemo(() => {
    const shortItems = new Set(shortfalls.map((s) => s.item));
    if (!shortItems.size) return [] as { id: Id; makes: string }[];
    const out: { id: Id; makes: string }[] = [];
    for (const g of factoryGroups) {
      const r = gamedata.recipes[g.recipe];
      if (r && r.ingredients.some(([ing]) => shortItems.has(ing)))
        out.push({ id: g.id, makes: name(r.products[0]?.[0] ?? g.recipe) });
    }
    return out;
  }, [shortfalls, factoryGroups, gamedata]);

  const blocked = shortfalls.length > 0;

  // The free-up confirm is a two-click latch. Reset it whenever the context
  // changes (different target/rate, or no longer blocked) so a stale "confirm"
  // can never fire a destructive delete on a single click in a new situation.
  useEffect(() => setConfirmFree(false), [target, rate, blocked]);

  const freeUp = async () => {
    if (!freeable.length) return;
    if (!confirmFree) {
      setConfirmFree(true);
      return;
    }
    setConfirmFree(false);
    await dispatch([...new Set(freeable.map((f) => f.id))].map((id) => ({ type: "delete_group", id }) as Command));
    pushToast(`Freed up ${shortfalls.map((s) => name(s.item)).join(", ")} — removed ${freeable.length} group(s).`, "success");
  };

  const build = async () => {
    if (!target || busy || blocked || !buildCp) return;
    setBusy(true);
    try {
      // first port carrying each raw (a factory may have several of the same).
      const portForItem = new Map<string, Id>();
      for (const p of inPorts) if (!portForItem.has(p.item)) portForItem.set(p.item, p.id);
      // reused intermediates → their existing group id (belts wire here, not a port).
      const reuseGroupOf = new Map<string, Id>();
      for (const item of reuseItems) {
        const prod = existingProducers.get(item);
        if (prod) reuseGroupOf.set(item, prod.id);
      }

      // column layout by topological depth, anchored right of the input ports.
      const baseX = Math.max(0, ...inPorts.map((p) => p.graphPos.x)) + 300;
      const maxDepth = Math.max(1, ...buildCp.groups.map((g) => g.depth));
      const byDepth = new Map<number, ChainGroup[]>();
      for (const g of buildCp.groups) byDepth.set(g.depth, [...(byDepth.get(g.depth) ?? []), g]);
      const posOf = new Map<string, { x: number; y: number }>();
      for (const [d, gs] of byDepth) {
        gs.forEach((g, i) => posOf.set(g.item, { x: baseX + (d - 1) * 300, y: 80 + i * 190 }));
      }

      const groupCmds: Command[] = buildCp.groups.map((g) => ({
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
        // Target the requested rate — the chain is sized for it, and the solve
        // is demand-driven, so a 0 target would idle every machine (all 0/min).
        rate: buildCp.rate,
        rateCeiling: null,
        graphPos: { x: baseX + maxDepth * 300, y: 80 },
      };

      const ids = await dispatch([...groupCmds, outCmd]);
      if (!ids) return;
      const groupId = new Map<string, Id>();
      buildCp.groups.forEach((g, i) => groupId.set(g.item, ids[i]));
      const outPortId = ids[buildCp.groups.length];

      // Scale reused groups only when their spare capacity can't absorb the new
      // draw: committed output (what they already feed) + the new demand vs the
      // group's current machine capacity. Reuses slack first, scales up if short.
      // Accumulate the MAX required count per group id, so a single group that
      // produces two reused items gets ONE set_group_count that satisfies both
      // (two commands in one dispatch would otherwise clobber each other).
      const needCountByGid = new Map<Id, number>();
      for (const [item, gid] of reuseGroupOf) {
        const prod = existingProducers.get(item)!;
        const newDemand = buildCp.belts
          .filter((b) => b.fromRaw && b.fromItem === item)
          .reduce((s, b) => s + b.rate, 0);
        const committed = derived.factories[factoryId]?.groups[gid]?.outRates[item] ?? 0;
        const capacityNow = prod.per * prod.count * prod.clock;
        const needed = committed + newDemand;
        if (needed > capacityNow + 1e-6) {
          const needCount = Math.ceil(needed / (prod.per * (prod.clock || 1)));
          needCountByGid.set(gid, Math.max(needCountByGid.get(gid) ?? 0, needCount));
        }
      }
      const scaleCmds: Command[] = [...needCountByGid].map(([id, count]) => ({
        type: "set_group_count",
        id,
        count,
      }));

      const edgeCmds: Command[] = buildCp.belts.map((b) => {
        const from: EdgeEnd = b.fromRaw
          ? reuseGroupOf.has(b.fromItem)
            ? { kind: "group", id: reuseGroupOf.get(b.fromItem)! }
            : { kind: "port", id: portForItem.get(b.fromItem)! }
          : { kind: "group", id: groupId.get(b.fromItem)! };
        const to: EdgeEnd =
          b.toItem === "OUT" ? { kind: "port", id: outPortId } : { kind: "group", id: groupId.get(b.toItem)! };
        return { type: "add_edge", factory: factoryId, from, to, item: b.item, tier: b.tier };
      });
      await dispatch([...scaleCmds, ...edgeCmds]);
      await dispatch([{ type: "tidy_layout", factory: factoryId }]).catch(() => {});

      setSelection(null);
      const reuseNote = reuseGroupOf.size
        ? ` — reused your ${[...reuseGroupOf.keys()].map(name).join(", ")}`
        : "";
      pushToast(
        `Built ${name(target)}${reuseNote} — ${buildCp.groups.length} new machine group(s).`,
        "success",
      );
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

            {reusable.length > 0 && (
              <label className="mfr-reuse" data-testid="mfr-reuse">
                <input type="checkbox" checked={reuseOn} onChange={(e) => setReuseOn(e.target.checked)} />
                <span>
                  Reuse &amp; extend your existing <b>{reusable.map(name).join(", ")}</b>{" "}
                  {reusable.length === 1 ? "line" : "lines"} instead of building{" "}
                  {reusable.length === 1 ? "a duplicate" : "duplicates"}.
                </span>
              </label>
            )}

            {blocked && (
              <div className="mfr-warn" data-testid="mfr-warn">
                <div className="mfr-warn-head mono">⚠ NOT ENOUGH EXTRACTION</div>
                <ul className="mfr-warn-list">
                  {shortfalls.map((s) => (
                    <li key={s.item}>
                      <b>{name(s.item)}</b>: this build needs {fmtRate(s.need)}/min but only {fmtRate(s.have)}
                      /min is free from your claimed nodes.
                    </li>
                  ))}
                </ul>
                <div className="mfr-warn-hint">
                  Claim another {shortfalls.map((s) => name(s.item)).join(" / ")} node nearby on the map
                  {feasibleRate >= 1 ? `, build at the ${feasibleRate}/min your nodes can feed,` : ""} or free
                  up the resource below.
                </div>
                {freeable.length > 0 && (
                  <button
                    className={`btn ${confirmFree ? "btn-danger" : "btn-ghost"} mfr-freeup`}
                    onClick={() => void freeUp()}
                    data-testid="mfr-freeup"
                  >
                    {confirmFree
                      ? `CONFIRM — REMOVE ${freeable.length} GROUP(S)`
                      : `FREE UP ${shortfalls.map((s) => name(s.item).toUpperCase()).join(" / ")} (REMOVE ${freeable.length} GROUP${freeable.length === 1 ? "" : "S"})`}
                  </button>
                )}
              </div>
            )}

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
              {blocked && feasibleRate >= 1 && (
                <button
                  className="btn btn-ghost"
                  disabled={busy}
                  onClick={() => setRate(feasibleRate)}
                  data-testid="mfr-reduce"
                >
                  BUILD AT {feasibleRate}/min
                </button>
              )}
              <button
                className="btn btn-primary"
                disabled={!target || busy || blocked}
                onClick={() => void build()}
                data-testid="mfr-build"
                title={blocked ? "Not enough extraction — see the warning above" : undefined}
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
