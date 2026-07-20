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

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../state/store";
import { fmtRate, itemLabel } from "../lib/format";
import ItemIcon from "../lib/ItemIcon";
import { POWER_ITEM, effClock, effCount, type Command, type EdgeEnd, type Id } from "../state/types";
import {
  makeableItems,
  planChain,
  planRawWiring,
  powerOptions,
  sizePowerBank,
  splitAcrossPorts,
  type ChainGroup,
  type ChainPlan,
  type RawConsumer,
  type RawWiring,
  type WiringRef,
} from "./makeChain";
import { minBeltTier } from "./logistics";

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
  // MAKE POWER (#119): generator burns runnable from these raws (coal → coal
  // power, etc.). Sized against the same pooled extraction headroom as items.
  const power = useMemo(() => powerOptions(gamedata, available), [gamedata, available]);

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

  // ADDED extraction demand of this build — what the capacity guard checks.
  // Fresh build: every raw feed of the fresh chain. With reuse ON, the fresh
  // total over-blocks: a reused line's feed comes from machines that ALREADY
  // draw their raws (counted in headroom), and redirecting its world exports
  // adds zero new extraction. So under reuse, count (a) the new groups' real
  // raw draw, plus (b) for each reused feed, only the remainder its
  // redirectable exports can't cover — expanded to raws via a sub-chain,
  // since scaling the reused line up pulls that much more from its inputs.
  // ADDED extraction demand for the picked item at an ARBITRARY target rate —
  // a pure function of the rate, so the capacity guard (at the live rate) and
  // the max-rate search (below, over many candidate rates) share one source of
  // truth. Under reuse this is AFFINE in the rate (a constant redirect credit),
  // not proportional, which is exactly why the max can't be a closed-form
  // rate × ratio and must be found by search.
  const rawDemandAt = useCallback(
    (r: number): Map<string, number> => {
      const d = new Map<string, number>();
      if (!target) return d;
      const fresh = planChain(gamedata, unlocked, available, target, r);
      const cp = reuseItems.size ? planChain(gamedata, unlocked, effectiveAvailable, target, r) : fresh;
      if (!cp) return d;
      for (const b of cp.belts) {
        if (!b.fromRaw || reuseItems.has(b.fromItem)) continue;
        d.set(b.fromItem, (d.get(b.fromItem) ?? 0) + b.rate);
      }
      // One expansion per reused GROUP: the build scales a group that feeds two
      // reused items ONCE (max count wins), so expanding each item's remainder
      // separately would double that group's raws — keep only the biggest.
      const extraByGid = new Map<Id, { item: string; extra: number; machines: number }>();
      for (const item of reuseItems) {
        const prod = existingProducers.get(item);
        if (!prod) continue;
        const newDemand = cp.belts
          .filter((b) => b.fromRaw && b.fromItem === item)
          .reduce((s, b) => s + b.rate, 0);
        if (newDemand <= 1e-6) continue;
        // Mirrors the build path's redirect: world exports THIS group feeds are
        // trimmable into the new chain without any new extraction — but a port
        // TARGET can exceed what the group really outputs (a starved export),
        // so credit no more than the solver's committed flow, exactly like the
        // build's min(freed, committed).
        const fedByGroup = (pid: Id) =>
          Object.values(plan.edges).some(
            (e) => e.from.kind === "group" && e.from.id === prod.id && e.to.kind === "port" && e.to.id === pid,
          );
        const redirectable = Object.values(plan.ports)
          .filter(
            (p) =>
              p.factory === factoryId &&
              p.direction === "out" &&
              p.item === item &&
              p.boundRoute === null &&
              p.rate > 0 &&
              fedByGroup(p.id),
          )
          .reduce((s, p) => s + p.rate, 0);
        const committed = derived.factories[factoryId]?.groups[prod.id]?.outRates[item] ?? 0;
        const extra = newDemand - Math.min(redirectable, committed);
        if (extra <= 1e-6) continue;
        const machines = extra / prod.per;
        const cur = extraByGid.get(prod.id);
        if (!cur || machines > cur.machines) extraByGid.set(prod.id, { item, extra, machines });
      }
      for (const { item, extra } of extraByGid.values()) {
        const sub = planChain(gamedata, unlocked, available, item, extra);
        if (sub) for (const b of sub.belts) if (b.fromRaw) d.set(b.fromItem, (d.get(b.fromItem) ?? 0) + b.rate);
      }
      return d;
    },
    [target, gamedata, unlocked, available, effectiveAvailable, reuseItems, existingProducers, plan.edges, plan.ports, factoryId, derived],
  );

  const rawDemand = useMemo(() => rawDemandAt(rate), [rawDemandAt, rate]);

  const shortfalls = useMemo(() => {
    const out: { item: string; need: number; have: number }[] = [];
    for (const [raw, need] of rawDemand) {
      const have = headroom.get(raw) ?? 0;
      if (need > have + 1e-6) out.push({ item: raw, need, have });
    }
    return out;
  }, [rawDemand, headroom]);

  // The MOST the assigned nodes can feed for the picked item. Raw draw is only
  // PROPORTIONAL to the rate for a from-scratch build; under reuse it's affine
  // (a constant redirect credit), so there is no closed-form rate × ratio — a
  // formula that reads the live rate would drift as the user edits it. Instead
  // find the largest integer rate whose draw fits every capped raw's headroom by
  // binary search over the pure rawDemandAt (draw is monotonic non-decreasing in
  // rate). This never reads the live `rate`, so it's a stable value that can't
  // oscillate the seeding effect. null = every raw is uncapped (supply assumed
  // unlimited) → no finite max. 0 = can't feed even 1/min.
  const maxRate = useMemo(() => {
    if (!target) return null;
    const fits = (r: number): { ok: boolean; capped: boolean } => {
      let capped = false;
      for (const [raw, need] of rawDemandAt(r)) {
        const h = headroom.get(raw) ?? Infinity;
        if (Number.isFinite(h)) capped = true;
        if (need > h + 1e-6) return { ok: false, capped };
      }
      return { ok: true, capped };
    };
    const at1 = fits(1);
    if (!at1.capped) return null; // unlimited supply — no finite ceiling
    if (!at1.ok) return 0; // can't feed even 1/min
    let hi = 1;
    while (hi < 1_000_000 && fits(hi * 2).ok) hi *= 2;
    let lo = hi;
    let up = Math.min(hi * 2, 1_000_000);
    while (up - lo > 1) {
      const mid = Math.floor((lo + up) / 2);
      if (fits(mid).ok) lo = mid;
      else up = mid;
    }
    return lo;
  }, [target, rawDemandAt, headroom]);

  // Unlimited-supply fallback: no finite max exists, so default to a single
  // final machine at 100% rather than an arbitrary constant — a clean, derived
  // starting point the user scales up (mirrors MAKE POWER defaulting to one
  // generator's nameplate when fuel headroom is unbounded).
  const oneMachineRate = useMemo(() => {
    if (!target) return null;
    const g = freshCp?.groups.find((x) => x.item === target);
    const r = g ? gamedata.recipes[g.recipe] : undefined;
    const prod = r?.products.find(([it]) => it === target);
    if (!r || !prod || r.durationS <= 0) return null;
    return Math.max(1, Math.round((prod[1] * 60) / r.durationS));
  }, [target, freshCp, gamedata.recipes]);

  // When blocked, the feasible build rate the nodes CAN sustain is exactly the
  // computed max (headroom-limited), surfaced by the warning's BUILD-AT button.
  const feasibleRate = maxRate ?? 0;

  // Seed the rate to that max (or the single-machine default when supply is
  // unlimited) — build as much as the claim supports, dial DOWN for less. Fires
  // exactly ONCE per item pick, tracked by a ref: never on a rate edit, a reuse
  // toggle, or a headroom change, so a manual entry is never clobbered.
  const seededFor = useRef<string | null>(null);
  useEffect(() => {
    if (!target) {
      seededFor.current = null;
      return;
    }
    if (seededFor.current === target) return;
    const seed = maxRate ?? oneMachineRate;
    if (seed != null && seed >= 1) {
      seededFor.current = target;
      setRate(seed);
    }
  }, [target, maxRate, oneMachineRate]);

  // Free-up: existing groups in THIS factory that consume a short raw.
  // Smallest draw first, and only as many as the gap needs — deleting every
  // consumer would over-free far beyond the shortfall. Draw is the solver's
  // REAL per-group intake (the same currency as headroom's used-draw), not
  // nameplate: an idle consumer draws nothing, so deleting it frees nothing
  // and it must never be picked. `covered` lists the raws whose gap the
  // selected groups actually close, so the toast can't over-claim.
  const freeable = useMemo(() => {
    const none = { groups: [] as { id: Id; makes: string }[], covered: [] as string[] };
    if (!shortfalls.length) return none;
    const dfGroups = derived.factories[factoryId]?.groups;
    const out = new Map<Id, string>();
    const covered: string[] = [];
    for (const s of shortfalls) {
      let remaining = s.need - s.have;
      const consumers = factoryGroups
        .flatMap((g) => {
          const r = gamedata.recipes[g.recipe];
          if (!r?.ingredients.some(([i]) => i === s.item)) return [];
          const draw = dfGroups?.[g.id]?.inRates[s.item] ?? 0;
          return draw > 1e-6 ? [{ g, r, draw }] : [];
        })
        .sort((a, b) => a.draw - b.draw);
      for (const c of consumers) {
        if (remaining <= 1e-6) break;
        if (!out.has(c.g.id)) out.set(c.g.id, name(c.r.products[0]?.[0] ?? c.g.recipe));
        remaining -= c.draw;
      }
      if (remaining <= 1e-6) covered.push(s.item);
    }
    return { groups: [...out].map(([id, makes]) => ({ id, makes })), covered };
  }, [shortfalls, factoryGroups, gamedata, derived, factoryId]);

  const blocked = shortfalls.length > 0;

  // The free-up confirm is a two-click latch. Reset it whenever the context
  // changes (different target/rate, or no longer blocked) so a stale "confirm"
  // can never fire a destructive delete on a single click in a new situation.
  useEffect(() => setConfirmFree(false), [target, rate, blocked]);

  // MAKE POWER: build a generator bank sized to the requested MW, fuel wired
  // through the same merger manifolds as item builds. maxMw per option is
  // what the pooled fuel headroom can burn.
  const maxMwOf = (opt: (typeof power)[number]) => {
    const h = headroom.get(opt.fuel) ?? 0;
    return h === Infinity ? Infinity : Math.floor((h / opt.fuelPer) * opt.mwPer);
  };
  const [mwInput, setMwInput] = useState<Record<string, number>>({});
  // The DISPLAYED MW for a row: user-entered, else pool max, else (uncapped
  // supply) one generator's nameplate. buildPower MUST build exactly this
  // number — a divergent build-side default once built MAX_SAFE_INTEGER MW
  // (~10^14 generators) on an uncapped port while the field showed "75".
  const displayMwOf = (opt: (typeof power)[number], maxMw: number) =>
    mwInput[opt.recipe] ?? (maxMw === Infinity ? opt.mwPer : maxMw);
  const buildPower = async (opt: (typeof power)[number]) => {
    if (busy) return;
    const maxMw = maxMwOf(opt);
    const requested = displayMwOf(opt, maxMw);
    const mw = maxMw === Infinity ? requested : Math.min(requested, maxMw);
    if (!(mw > 0)) return;
    setBusy(true);
    try {
      const { count, clock, fuelNeed } = sizePowerBank(opt, mw);
      const df = derived.factories[factoryId];
      const pool = inPorts
        .filter((p) => p.item === opt.fuel)
        .map((p) => ({
          id: p.id,
          left: p.rateCeiling == null ? Infinity : Math.max(0, p.rateCeiling - (df?.ports[p.id] ?? 0)),
        }));
      const shares = splitAcrossPorts(pool, fuelNeed);
      if (!shares.length) return;
      const wiring = planRawWiring(shares, [{ key: "bank", rate: fuelNeed }]);

      const baseX = Math.max(0, ...inPorts.map((p) => p.graphPos.x)) + 300;
      const cmds: Command[] = [
        { type: "add_group", factory: factoryId, machine: opt.machine, recipe: opt.recipe, count, clock, graphPos: { x: baseX, y: 80 }, floor: 0 },
        ...wiring.junctions.map(
          (j, i): Command => ({
            type: "add_junction",
            factory: factoryId,
            kind: j.kind,
            graphPos: { x: baseX - 180, y: 80 + i * 110 },
            floor: 0,
          }),
        ),
      ];
      const ids = await dispatch(cmds);
      if (!ids) return;
      const bankId = ids[0];
      const junctionId = new Map(wiring.junctions.map((j, i) => [j.key, ids[1 + i]]));
      const end = (r: WiringRef): EdgeEnd =>
        r.kind === "port"
          ? { kind: "port", id: r.id }
          : r.kind === "junction"
            ? { kind: "junction", id: junctionId.get(r.key)! }
            : { kind: "group", id: bankId };
      await dispatch(
        wiring.edges.map(
          (e): Command => ({
            type: "add_edge",
            factory: factoryId,
            from: end(e.from),
            to: end(e.to),
            item: opt.fuel,
            tier: minBeltTier(e.rate),
          }),
        ),
      );
      await dispatch([{ type: "tidy_layout", factory: factoryId }]).catch(() => {});
      setSelection(null);
      pushToast(
        `Built ${count} × ${gamedata.machines[opt.machine]?.displayName ?? "generator"} — ⚡ ${Math.round(mw)} MW from ${name(opt.fuel)}.`,
        "success",
      );
      onClose();
    } finally {
      setBusy(false);
    }
  };

  const freeUp = async () => {
    if (!freeable.groups.length) return;
    if (!confirmFree) {
      setConfirmFree(true);
      return;
    }
    setConfirmFree(false);
    await dispatch(freeable.groups.map((f) => ({ type: "delete_group", id: f.id }) as Command));
    // Claim only the raws the removed draw actually covers — an uncoverable
    // gap (or a raw with no live consumers) is not "freed up".
    pushToast(
      freeable.covered.length
        ? `Freed up ${freeable.covered.map(name).join(", ")} — removed ${freeable.groups.length} group(s).`
        : `Removed ${freeable.groups.length} group(s) — not enough consumer draw to cover the shortfall.`,
      freeable.covered.length ? "success" : "info",
    );
  };

  const build = async () => {
    if (!target || busy || blocked || !buildCp) return;
    setBusy(true);
    try {
      // ALL ports carrying each raw, with their remaining headroom — a factory
      // with two claims of the same resource must draw from BOTH (the guard
      // sums them; wiring everything to the first port would starve the chain
      // at one node's ceiling while the second sits idle).
      const df = derived.factories[factoryId];
      const portPool = new Map<string, { id: Id; left: number }[]>();
      for (const p of inPorts) {
        const used = df?.ports[p.id] ?? 0;
        const left = p.rateCeiling == null ? Infinity : Math.max(0, p.rateCeiling - used);
        portPool.set(p.item, [...(portPool.get(p.item) ?? []), { id: p.id, left }]);
      }
      // reused intermediates → their existing group id (belts wire here, not a port).
      const reuseGroupOf = new Map<string, Id>();
      for (const item of reuseItems) {
        const prod = existingProducers.get(item);
        if (prod) reuseGroupOf.set(item, prod.id);
      }

      // Raw supply wiring: allocate each raw's TOTAL demand across its ports,
      // then route through real merger/splitter junctions (#94/#97) exactly as
      // a hand build would — mergers combine multiple claims into one stream,
      // splitters fan one stream out to multiple consumers. A genuine 1:1
      // stays a plain belt.
      const rawConsumers = new Map<string, RawConsumer[]>();
      for (const b of buildCp.belts) {
        if (!b.fromRaw || reuseGroupOf.has(b.fromItem) || b.toItem === "OUT") continue;
        rawConsumers.set(b.fromItem, [...(rawConsumers.get(b.fromItem) ?? []), { key: b.toItem, rate: b.rate }]);
      }
      const rawWirings = new Map<string, RawWiring>();
      for (const [item, consumers] of rawConsumers) {
        const total = consumers.reduce((s, c) => s + c.rate, 0);
        const shares = splitAcrossPorts(portPool.get(item) ?? [], total);
        if (shares.length) rawWirings.set(item, planRawWiring(shares, consumers));
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

      // Junctions for the raw manifolds — created with the groups so the edge
      // pass can reference their ids. Rough positions between the ports and
      // the first machine column; tidy_layout re-places everything at the end.
      const junctionMeta: { item: string; key: string }[] = [];
      const junctionCmds: Command[] = [];
      for (const [item, w] of rawWirings) {
        for (const j of w.junctions) {
          junctionMeta.push({ item, key: j.key });
          junctionCmds.push({
            type: "add_junction",
            factory: factoryId,
            kind: j.kind,
            graphPos: { x: baseX - (j.kind === "merger" ? 180 : 90), y: 80 + junctionCmds.length * 110 },
            floor: 0,
          });
        }
      }

      const ids = await dispatch([...groupCmds, outCmd, ...junctionCmds]);
      if (!ids) return;
      const groupId = new Map<string, Id>();
      buildCp.groups.forEach((g, i) => groupId.set(g.item, ids[i]));
      const outPortId = ids[buildCp.groups.length];
      const junctionId = new Map<string, Id>();
      junctionMeta.forEach((m, i) => junctionId.set(`${m.item}:${m.key}`, ids[buildCp.groups.length + 1 + i]));

      // Scale reused groups only when their spare capacity can't absorb the new
      // draw: committed output (what they already feed) + the new demand vs the
      // group's current machine capacity. Reuses slack first, scales up if short.
      // Accumulate the MAX required count per group id, so a single group that
      // produces two reused items gets ONE set_group_count that satisfies both
      // (two commands in one dispatch would otherwise clobber each other).
      const needCountByGid = new Map<Id, number>();
      const portRateCmds: Command[] = [];
      let redirected = false;
      for (const [item, gid] of reuseGroupOf) {
        const prod = existingProducers.get(item)!;
        const newDemand = buildCp.belts
          .filter((b) => b.fromRaw && b.fromItem === item)
          .reduce((s, b) => s + b.rate, 0);
        // Auto-feed downstream: a reused intermediate was likely fully exported
        // to the world by an earlier MAKE. Redirect that export into the new
        // chain by trimming its world OUT port target(s) by the new internal
        // demand — otherwise the export keeps eating the whole output and the
        // new consumer starves (idle belts). Non-destructive: the port stays,
        // just retargeted; raise it back to export surplus again.
        // Only ports THIS group actually feeds (a belt group→port exists) are
        // eligible — a same-item export fed by a different producer is not ours
        // to trim, and freeing it would both cut that export wrongly and skip
        // the scale-up the reused group still needs.
        const fedByGroup = (pid: Id) =>
          Object.values(plan.edges).some(
            (e) => e.from.kind === "group" && e.from.id === gid && e.to.kind === "port" && e.to.id === pid,
          );
        const worldPorts = Object.values(plan.ports).filter(
          (p) =>
            p.factory === factoryId &&
            p.direction === "out" &&
            p.item === item &&
            p.boundRoute === null &&
            p.rate > 0 &&
            fedByGroup(p.id),
        );
        let toFree = newDemand;
        let freed = 0;
        for (const p of worldPorts) {
          if (toFree <= 1e-6) break;
          const cut = Math.min(p.rate, toFree);
          if (cut > 1e-6) {
            portRateCmds.push({ type: "set_port_rate", id: p.id, rate: p.rate - cut });
            toFree -= cut;
            freed += cut;
            redirected = true;
          }
        }
        const committed = derived.factories[factoryId]?.groups[gid]?.outRates[item] ?? 0;
        const capacityNow = prod.per * prod.count * prod.clock;
        // Output needed after the redirect: current output, minus the export we
        // just freed, plus the new internal demand. `committed` is the solver's
        // actual (capacity-clamped) output while the trim works on port TARGETS,
        // so credit no more freed output than was really flowing — an
        // under-provisioned line must still scale up to cover the new draw.
        const needed = committed - Math.min(freed, committed) + newDemand;
        if (needed > capacityNow + 1e-6) {
          const needCount = Math.ceil(needed / (prod.per * (prod.clock || 1)));
          needCountByGid.set(gid, Math.max(needCountByGid.get(gid) ?? 0, needCount));
        }
      }
      const scaleCmds: Command[] = [
        ...portRateCmds,
        ...[...needCountByGid].map((entry): Command => ({ type: "set_group_count", id: entry[0], count: entry[1] })),
      ];

      const refEnd = (item: string, r: WiringRef): EdgeEnd =>
        r.kind === "port"
          ? { kind: "port", id: r.id }
          : r.kind === "junction"
            ? { kind: "junction", id: junctionId.get(`${item}:${r.key}`)! }
            : { kind: "group", id: groupId.get(r.key)! };
      const edgeCmds: Command[] = [
        // raw supply, through the planned merger/splitter manifolds
        ...[...rawWirings].flatMap(([item, w]) =>
          w.edges.map(
            (e): Command => ({
              type: "add_edge",
              factory: factoryId,
              from: refEnd(item, e.from),
              to: refEnd(item, e.to),
              item,
              tier: minBeltTier(e.rate),
            }),
          ),
        ),
        // produced intermediates, reused feeds, and the final OUT belt
        ...buildCp.belts
          .filter((b) => !(b.fromRaw && !reuseGroupOf.has(b.fromItem)))
          .map((b): Command => {
            const from: EdgeEnd = b.fromRaw
              ? { kind: "group", id: reuseGroupOf.get(b.fromItem)! }
              : { kind: "group", id: groupId.get(b.fromItem)! };
            const to: EdgeEnd =
              b.toItem === "OUT" ? { kind: "port", id: outPortId } : { kind: "group", id: groupId.get(b.toItem)! };
            return { type: "add_edge", factory: factoryId, from, to, item: b.item, tier: b.tier };
          }),
      ];
      await dispatch([...scaleCmds, ...edgeCmds]);
      await dispatch([{ type: "tidy_layout", factory: factoryId }]).catch(() => {});

      setSelection(null);
      const reuseNote = reuseGroupOf.size
        ? ` — reused your ${[...reuseGroupOf.keys()].map(name).join(", ")}${redirected ? " (redirected from world export to feed this chain)" : ""}`
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

  // MAKE POWER rows: one per burnable fuel — MW input defaults to what the
  // pooled nodes can feed; over-typing offers "build at max" instead of a
  // silent clamp. Rendered whether or not any ITEM is makeable (a coal-only
  // factory makes nothing, but it burns).
  const powerSection =
    power.length > 0 ? (
      <div className="mfr-power" data-testid="mfr-power">
        <div className="mfr-power-head mono">⚡ MAKE POWER</div>
        {power.map((opt) => {
          const maxMw = maxMwOf(opt);
          const mw = displayMwOf(opt, maxMw);
          const over = maxMw !== Infinity && mw > maxMw;
          return (
            <div key={opt.recipe} className="mfr-power-row" data-testid={`mfr-power-${opt.fuel}`}>
              <ItemIcon item={opt.fuel} displayName={name(opt.fuel)} size={28} />
              <div className="mfr-power-info">
                <span className="mfr-power-name">
                  {gamedata.machines[opt.machine]?.displayName ?? opt.machine}
                </span>
                <span className="mfr-power-sub mono">
                  ⚡ {opt.mwPer} MW · {fmtRate(opt.fuelPer)}/min {name(opt.fuel)} each
                  {maxMw !== Infinity ? ` · your nodes feed up to ${maxMw} MW` : ""}
                </span>
              </div>
              <input
                type="number"
                min={0}
                className="mono mfr-power-mw"
                value={Math.round(mw)}
                onChange={(e) =>
                  setMwInput((m) => ({ ...m, [opt.recipe]: Math.max(1, Number(e.target.value) || 1) }))
                }
                data-testid={`mfr-power-mw-${opt.fuel}`}
              />
              <span className="unit mono">MW</span>
              <button
                className="btn btn-primary"
                disabled={busy || maxMw < 1}
                onClick={() => void buildPower(opt)}
                data-testid={`mfr-power-build-${opt.fuel}`}
                title={maxMw < 1 ? "No fuel headroom left on these nodes" : undefined}
              >
                {over ? `BUILD AT ${maxMw} MW` : "BUILD"}
              </button>
            </div>
          );
        })}
        <div className="mfr-power-note">
          In-game coal/fuel plants also need piped water — pipes aren't modeled here, so plan
          extractors separately.
        </div>
      </div>
    ) : null;

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
          <>
            <div className="mfr-empty">
              {power.some((o) => maxMwOf(o) >= 1)
                ? "No items are fully makeable from these inputs alone — but they can BURN. Build power below, or add more raws (e.g. another ore) to unlock recipes."
                : power.length > 0
                  ? "No items are fully makeable, and the fuel here is already fully committed — claim more nodes to build items or power."
                  : "Nothing is fully makeable from these inputs alone. Add more raw resources (e.g. another ore) to unlock recipes."}
            </div>
            {powerSection}
          </>
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
                {freeable.groups.length > 0 && (
                  <button
                    className={`btn ${confirmFree ? "btn-danger" : "btn-ghost"} mfr-freeup`}
                    onClick={() => void freeUp()}
                    data-testid="mfr-freeup"
                  >
                    {confirmFree
                      ? `CONFIRM — REMOVE ${freeable.groups.length} GROUP(S)`
                      : `FREE UP ${shortfalls.map((s) => name(s.item).toUpperCase()).join(" / ")} (REMOVE ${freeable.groups.length} GROUP${freeable.groups.length === 1 ? "" : "S"})`}
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
              {/* Defaults to the most the nodes can feed; this snaps back to it
                  after a dial-down and doubles as the "your nodes feed up to N"
                  readout. Hidden when over-typed into a shortfall — the warning's
                  BUILD-AT button owns that case. */}
              {!blocked && maxRate != null && maxRate >= 1 && (
                <button
                  className="btn btn-ghost mfr-max"
                  disabled={busy || rate >= maxRate}
                  onClick={() => setRate(maxRate)}
                  data-testid="mfr-max"
                  title="Set to the most your claimed nodes can feed"
                >
                  MAX {maxRate}/min
                </button>
              )}
              {/* No finite ceiling: every raw input is uncapped (assumed
                  unlimited). The field seeded to one full machine — say so, so
                  the absence of a MAX button reads as "unbounded", not a bug. */}
              {target && maxRate === null && (
                <span className="mfr-max-hint mono" data-testid="mfr-unlimited">
                  supply assumed unlimited — set any rate
                </span>
              )}
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
            {/* AFTER the item footer/warning: a blocked-ITEM warning sitting
                between the power rows and their BUILD buttons read as a power
                shortfall. Power is its own self-contained block down here. */}
            {powerSection}
          </>
        )}
      </div>
    </div>
  );
}
