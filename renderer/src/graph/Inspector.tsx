// Inspector (mock 4a, right, 360px). OUTPUT TARGET slider re-solves live:
// T0 (WASM) on drag frames rendering italic projections; T1 on release with a
// settle flash. The slider hard-stops at the binding constraint and names it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore, solveChip } from "../state/store";
import { buildSnapshot, ensureT0, t0SetTarget } from "../solver/t0";
import { footprintFor, footprintArea } from "./footprints";
import { fmtClock, fmtPower, fmtRate, flowBand, bottleneckEdges, itemLabel } from "../lib/format";
import {
  clampEdgeTier,
  effClock,
  effCount,
  isFluidItem,
  POWER_ITEM,
  transportCapacity,
  transportTiers,
  type DerivedFactory,
  type Id,
} from "../state/types";
import ItemIcon from "../lib/ItemIcon";
import { groupLogistics, balancedJunctions, SPLITTER_CLASS, MERGER_CLASS } from "./logistics";
import SendToFactory from "./SendToFactory";
import ReceiveFromFactory from "./ReceiveFromFactory";

const CLOCK_STEPS = [0.5, 0.75, 1.0, 1.5, 2.5];
// Satisfactory overclock power exponent (power ∝ clock^k) — for the
// consolidation tip's estimated MW delta.
const POWER_EXP = 1.321928;

export default function Inspector({
  factoryId,
  df,
  isProjected,
}: {
  factoryId: Id;
  df: DerivedFactory | undefined;
  isProjected: boolean;
}) {
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const selection = useStore((s) => s.selection);
  const derived = useStore((s) => s.derived);
  const dispatch = useStore((s) => s.dispatch);
  const setSelection = useStore((s) => s.setSelection);
  const setProjected = useStore((s) => s.setProjected);

  const factory = plan.factories[factoryId];
  const outPort = factory?.ports.map((id) => plan.ports[id]).find((p) => p?.direction === "out");
  const authoritative = derived.factories[factoryId];
  const ceiling = authoritative?.targetCeiling ?? null;

  // Slider state: value tracks canonical rate except mid-drag.
  const [dragValue, setDragValue] = useState<number | null>(null);
  // Manual entry: while the field is focused/edited, `numText` holds the raw
  // string so typing isn't fought by re-renders; committed on blur/Enter.
  const [numText, setNumText] = useState<string | null>(null);
  const dragging = dragValue !== null;
  const rate = dragging ? dragValue : outPort?.rate ?? 0;
  const wasmReady = useRef(false);
  // Inter-factory supply modals, launched from a selected boundary port:
  // send (from an OUT port) and its mirror, receive (from an IN port).
  const [sendingFrom, setSendingFrom] = useState<Id | null>(null);
  const [receivingInto, setReceivingInto] = useState<Id | null>(null);

  useEffect(() => {
    void ensureT0().then(() => {
      wasmReady.current = true;
    });
  }, []);

  const sliderMax = useMemo(() => {
    const base = ceiling ? ceiling.maxRate : Math.max(10, (outPort?.rate ?? 0) * 2);
    return Math.max(1, base);
  }, [ceiling, outPort?.rate]);

  // Optimal-value breakpoints: rates at whole-machine counts (each machine at
  // 100% on the final recipe) plus the input ceiling. Sliding/typing snaps to
  // these clean values so the user lands on a build that runs full-clock rather
  // than a fractional one. Derived from the group that outputs this port's item.
  const breakpoints = useMemo(() => {
    if (!outPort || outPort.item === POWER_ITEM) return [] as number[];
    const finalGroup = Object.values(plan.groups).find(
      (g) => g.factory === factoryId && gamedata.recipes[g.recipe]?.products.some(([it]) => it === outPort.item),
    );
    const perMachine = finalGroup
      ? gamedata.recipes[finalGroup.recipe]?.products.find(([it]) => it === outPort.item)?.[1] ?? 0
      : 0;
    if (perMachine <= 0) return [] as number[];
    const cap = ceiling ? ceiling.maxRate : sliderMax;
    const pts: number[] = [];
    for (let k = 1; k * perMachine <= cap + 1e-6 && pts.length < 16; k++) pts.push(k * perMachine);
    if (ceiling && !pts.some((p) => Math.abs(p - ceiling.maxRate) < 1e-6)) pts.push(ceiling.maxRate);
    return pts.sort((a, b) => a - b);
  }, [outPort, plan.groups, factoryId, gamedata.recipes, ceiling, sliderMax]);

  const snapToBreakpoint = useCallback(
    (v: number) => {
      const thresh = sliderMax * 0.02; // within 2% of a clean value → snap to it
      for (const bp of breakpoints) if (Math.abs(bp - v) < thresh) return bp;
      return v;
    },
    [breakpoints, sliderMax],
  );

  // Live T0 projection for a candidate target (shared by drag, typing, chips).
  const project = useCallback(
    (clamped: number) => {
      if (!outPort || !wasmReady.current) return;
      const snapshot = buildSnapshot(useStore.getState().plan, useStore.getState().gamedata, factoryId);
      if (!snapshot) return;
      const result = t0SetTarget(snapshot, outPort.id, clamped);
      if (result) setProjected({ factoryId, result, targetRate: clamped });
    },
    [outPort, factoryId, setProjected],
  );

  const clampTarget = useCallback(
    (v: number) => {
      let c = Math.max(0, ceiling ? Math.min(v, ceiling.maxRate) : v);
      if (ceiling && c > ceiling.maxRate - sliderMax * 0.01) c = ceiling.maxRate;
      return snapToBreakpoint(c);
    },
    [ceiling, sliderMax, snapToBreakpoint],
  );

  const onDrag = useCallback(
    (v: number) => {
      if (!outPort) return;
      const clamped = clampTarget(v);
      setDragValue(clamped);
      project(clamped);
    },
    [outPort, clampTarget, project],
  );

  // Commit a target directly (numeric entry / breakpoint chip): project + persist.
  const commitTarget = useCallback(
    (v: number) => {
      if (!outPort) return;
      const clamped = clampTarget(v);
      setDragValue(null);
      project(clamped);
      void dispatch([{ type: "set_port_rate", id: outPort.id, rate: clamped }]);
    },
    [outPort, clampTarget, project, dispatch],
  );

  const onRelease = useCallback(() => {
    if (!outPort || dragValue === null) return;
    const v = dragValue;
    setDragValue(null);
    void dispatch([{ type: "set_port_rate", id: outPort.id, rate: v }]);
  }, [outPort, dragValue, dispatch]);

  const selectedGroup = selection?.kind === "group" ? plan.groups[selection.id] : null;
  const selectedJunction = selection?.kind === "junction" ? plan.junctions[selection.id] : null;
  const selectedEdge = selection?.kind === "edge" ? plan.edges[selection.id] : null;
  const selectedPort = selection?.kind === "port" ? plan.ports[selection.id] : null;
  // Tier label for an edge option/row: "PIPE Mk.n" for fluids, "MK.n" for belts.
  // Clamps a stale belt tier (a legacy fluid edge) into range so the row/select
  // value matches an offered option — pipes reach Mk.2.
  const edgeTierLabel = (item: string, tier: number) => {
    const t = clampEdgeTier(gamedata, item, tier);
    return isFluidItem(gamedata, item) ? `PIPE Mk.${t}` : `MK.${t}`;
  };
  // The factory-level OUTPUT TARGET (+ its binding-constraint warning) is the
  // overview control. When a specific belt / group / junction — or a non-output
  // port — is selected, that entity's own sections own the panel; showing the
  // factory target too reads as if it belongs to the selection (e.g. a CABLE
  // target and a BIOMASS-belt binding rendered over a selected IRON ROD belt).
  // The inspector only mounts for a group / port / edge selection, so the factory
  // OUTPUT TARGET renders exactly when the out port itself is selected — never
  // over a machine, belt, or a different port, which each own the panel. (The
  // group/edge/junction guards are belt-and-suspenders against future mounts.)
  const showFactoryTarget =
    !!outPort && !selectedGroup && !selectedEdge && !selectedJunction && (!selectedPort || selectedPort.id === outPort.id);
  const chip = solveChip(authoritative);
  const atCeiling = !!ceiling && rate >= ceiling.maxRate - 1e-6;

  const bindingText = useMemo(() => {
    if (!ceiling) return null;
    const b = ceiling.binding;
    const itemName = itemLabel(gamedata.items, b.item);
    if (b.kind === "belt_capacity") {
      return { text: `${itemName.toUpperCase()} BELT AT ${fmtRate(b.capacity)}/MIN`, fix: "UPGRADE BELT TIER" };
    }
    if (b.kind === "input_ceiling") {
      return { text: `${itemName.toUpperCase()} INPUT CEILING ${fmtRate(b.ceiling)}/MIN`, fix: "RAISE EXTRACTION" };
    }
    return { text: `${itemName.toUpperCase()} NOT WIRED`, fix: "CONNECT THE MISSING BELT" };
  }, [ceiling, gamedata.items]);

  const dgSel = selectedGroup ? df?.groups[selectedGroup.id] : null;
  // One footprint lookup per render; "clearance" = the game's own Docs
  // clearance data, "est." = the community fallback table (same provenance
  // grammar as the card strip's tooltip).
  const fp = selectedGroup ? footprintFor(gamedata, selectedGroup.machine) : null;
  const feedEdges = selectedGroup
    ? Object.values(plan.edges).filter((e) => e.to.kind === "group" && e.to.id === selectedGroup.id)
    : [];
  // Solver-named capacity bindings — the honest bottleneck red (efficiency
  // grammar: a full feed belt that keeps its machines fed is optimal).
  const bottlenecks = useMemo(() => bottleneckEdges(df), [df]);

  // Belt-logistics detail for the selected bank: splitter/merger counts (both
  // layouts), per-line belt tiers, and efficiency tips.
  const groupLogi = useMemo(() => {
    if (!selectedGroup) return null;
    const n = effCount(selectedGroup);
    if (n <= 1) return null;
    const inR = dgSel?.inRates ?? {};
    const outR = Object.fromEntries(Object.entries(dgSel?.outRates ?? {}).filter(([it]) => it !== POWER_ITEM));
    const logi = groupLogistics(n, inR, outR, (item) => isFluidItem(gamedata, item));
    if (logi.splitters.balanced === 0 && logi.mergers.balanced === 0) return null;
    const clk = effClock(selectedGroup);
    const tips: string[] = [];
    for (const l of [...logi.inputs, ...logi.outputs]) {
      const name = itemLabel(gamedata.items, l.item);
      const carrier = l.fluid ? "pipe" : "belt";
      const topTier = l.fluid ? "Mk.2 pipes" : "Mk.6 belts";
      if (l.lines > 1) tips.push(`${name}: ${fmtRate(l.rate)}/min needs ${l.lines} parallel ${topTier}.`);
      else if (l.tier > 1) tips.push(`${name}: ${fmtRate(l.rate)}/min needs a Mk.${l.tier} ${carrier}.`);
    }
    // Consolidation: same output from fewer machines at higher clock (≤250%) —
    // fewer junctions, but more power (power ∝ clock^k).
    const work = n * clk;
    const minM = Math.max(1, Math.ceil(work / 2.5));
    if (minM < n) {
      const newClk = work / minM;
      const curP = dgSel?.powerMw ?? 0;
      const base = curP > 0 ? curP / (n * Math.pow(clk, POWER_EXP)) : 0;
      const dMW = base > 0 ? base * minM * Math.pow(newClk, POWER_EXP) - curP : 0;
      const savedJ =
        logi.splitters.balanced +
        logi.mergers.balanced -
        (logi.inputs.length + logi.outputs.length) * balancedJunctions(minM);
      tips.push(
        `${n} @ ${Math.round(clk * 100)}% → ${minM} @ ${Math.round(newClk * 100)}%: ` +
          `−${n - minM} machine${n - minM === 1 ? "" : "s"}, −${savedJ} junction${savedJ === 1 ? "" : "s"}` +
          (base > 0 ? ` · ≈ +${fmtPower(dMW)} power` : ""),
      );
    }
    return { logi, tips };
  }, [selectedGroup, dgSel, gamedata.items]);

  return (
    <aside className="inspector" data-testid="inspector">
      {/* ---- OUTPUT TARGET (factory-level) ---- */}
      {showFactoryTarget && (
        <section className="insp-section">
          <h3 className="t-label">OUTPUT TARGET — {itemLabel(gamedata.items, outPort.item).toUpperCase()}</h3>
          <div className="insp-target-row">
            <span
              className={`t-data-22 ${dragging || isProjected ? "projected" : ""}`}
              data-testid="target-value"
            >
              {outPort.item === POWER_ITEM ? (
                fmtPower(df?.ports[outPort.id] ?? rate)
              ) : (
                <>
                  {fmtRate(df?.ports[outPort.id] ?? rate)}
                  <span className="unit">/min</span>
                </>
              )}
            </span>
            {outPort.item !== POWER_ITEM && (
              // Type an exact target instead of dragging; commits (and snaps to a
              // clean value) on Enter or blur.
              <span className="insp-target-entry mono">
                <input
                  type="number"
                  className="insp-target-input"
                  min={0}
                  max={ceiling ? Math.ceil(ceiling.maxRate) : undefined}
                  step="any"
                  value={numText ?? String(Math.round(rate * 10) / 10)}
                  onFocus={(e) => e.currentTarget.select()}
                  onChange={(e) => setNumText(e.target.value)}
                  onBlur={() => {
                    if (numText !== null) {
                      const v = Number(numText);
                      if (Number.isFinite(v)) commitTarget(v);
                      setNumText(null);
                    }
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                  }}
                  data-testid="target-input"
                  aria-label="Set output target per minute"
                />
                <span className="unit">/min</span>
              </span>
            )}
          </div>
          <div className="insp-slider-wrap">
            <input
              type="range"
              className="insp-slider"
              min={0}
              max={sliderMax}
              step={sliderMax / 200}
              value={rate}
              list="insp-breakpoints"
              onChange={(e) => onDrag(Number(e.target.value))}
              onPointerUp={onRelease}
              onKeyUp={(e) => {
                if (e.key === "ArrowLeft" || e.key === "ArrowRight") onRelease();
              }}
              data-testid="target-slider"
            />
            <datalist id="insp-breakpoints">
              {breakpoints.map((bp, i) => (
                <option key={i} value={bp} />
              ))}
            </datalist>
            {ceiling && (
              <span
                className="insp-tick"
                style={{ left: `${Math.min(100, (ceiling.maxRate / sliderMax) * 100)}%` }}
                title={`Input ceiling: ${fmtRate(ceiling.maxRate)}/min`}
              />
            )}
          </div>
          <div className="insp-slider-bounds mono">
            <span>0</span>
            {ceiling && <span className="insp-ceiling-note">▲ CEILING {fmtRate(ceiling.maxRate)}/min</span>}
            <span>{fmtRate(sliderMax)}</span>
          </div>
          {breakpoints.length > 1 && (
            // Optimal-value snap chips: each whole-machine count at full clock,
            // plus the ceiling. Clicking jumps the target straight to it.
            <div className="insp-breakpoints" data-testid="target-breakpoints">
              <span className="insp-bp-label mono">SNAP</span>
              {(breakpoints.length > 6
                ? [...breakpoints.slice(0, 5), breakpoints[breakpoints.length - 1]]
                : breakpoints
              ).map((bp, i) => {
                const active = Math.abs(rate - bp) < sliderMax * 0.005;
                const isCeil = !!ceiling && Math.abs(bp - ceiling.maxRate) < 1e-6;
                return (
                  <button
                    key={i}
                    className={`insp-bp-chip mono ${active ? "active" : ""} ${isCeil ? "ceil" : ""}`}
                    onClick={() => commitTarget(bp)}
                    title={isCeil ? "input ceiling" : "runs the machines at full clock"}
                  >
                    {fmtRate(bp)}
                  </button>
                );
              })}
            </div>
          )}
          <div className="insp-solvenote mono">
            RE-SOLVES LIVE ON DRAG · {chip.text}
            {authoritative?.solveOnRelease && " · LIVE → ON RELEASE"}
          </div>
          {atCeiling && bindingText && (
            <div className="insp-binding" data-testid="binding-strip">
              <span className="mono">⛔ {bindingText.text}</span>
              <span className="insp-binding-fix t-label">{bindingText.fix}</span>
            </div>
          )}
        </section>
      )}

      {/* ---- selected machine group ---- */}
      {selectedGroup && (
        <>
          <section className="insp-section">
            <h3 className="t-label">
              CLOCK — {gamedata.machines[selectedGroup.machine]?.displayName?.toUpperCase()} ×{selectedGroup.count}
            </h3>
            <div className="insp-clock-row">
              {CLOCK_STEPS.map((c) => (
                <button
                  key={c}
                  className={`insp-clock-btn mono ${Math.abs(effClock(selectedGroup) - c) < 1e-9 ? "active" : ""}`}
                  onClick={() => void dispatch([{ type: "set_group_clock", id: selectedGroup.id, clock: c }])}
                >
                  {c * 100}
                </button>
              ))}
              <input
                className="insp-clock-fine mono"
                key={selectedGroup.id + effClock(selectedGroup)}
                defaultValue={fmtClock(effClock(selectedGroup))}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  const v = parseFloat(e.currentTarget.value) / 100;
                  if (isFinite(v) && v > 0)
                    void dispatch([{ type: "set_group_clock", id: selectedGroup.id, clock: Math.min(2.5, Math.max(0.01, v)) }]);
                }}
              />
            </div>
            <div className="insp-note">Above 100% needs power shards in-game — the plan records intent.</div>
          </section>

          <section className="insp-section">
            <h3 className="t-label">PLACEMENT</h3>
            <div className="drawer-row">
              <span className="drawer-row-name">Floor</span>
              <div className="floor-stepper" data-testid="floor-stepper">
                <button
                  className="insp-clock-btn mono"
                  disabled={selectedGroup.floor === 0}
                  onClick={() =>
                    void dispatch([{ type: "set_group_floor", id: selectedGroup.id, floor: selectedGroup.floor - 1 }])
                  }
                >
                  −
                </button>
                <span className="t-data-12 floor-value">F{selectedGroup.floor}</span>
                <button
                  className="insp-clock-btn mono"
                  onClick={() =>
                    void dispatch([{ type: "set_group_floor", id: selectedGroup.id, floor: selectedGroup.floor + 1 }])
                  }
                >
                  +
                </button>
              </div>
            </div>
            {fp && (
              <div className="drawer-row">
                <span className="drawer-row-name">Footprint</span>
                <span
                  className="t-data-12"
                  title="Top-down clearance pad (build + approach), not wall-to-wall dims"
                >
                  {fp.w} × {fp.l} m {fp.derived ? "clearance" : "est."} · ×{selectedGroup.count} →{" "}
                  {fmtRate(footprintArea(fp, selectedGroup.count))} m²
                </span>
              </div>
            )}
            <div className="insp-note">
              Belts to other floors render as lifts (⇅). Footprints are per-machine clearance pads
              (build + approach), top-down — not wall-to-wall dims.
            </div>
          </section>

          <section className="insp-section">
            <h3 className="t-label">I/O</h3>
            {gamedata.recipes[selectedGroup.recipe]?.ingredients.map(([item]) => {
              const rate = dgSel?.inRates[item] ?? 0;
              const feeding = feedEdges.filter((e) => e.item === item);
              const cap = feeding.reduce((acc, e) => acc + transportCapacity(gamedata, e.item, e.tier), 0);
              const sat = cap > 0 ? rate / cap : 0;
              const band = flowBand(sat, rate, feeding.some((e) => bottlenecks.has(e.id)));
              return (
                <div className="drawer-row" key={item}>
                  <ItemIcon item={item} displayName={gamedata.items[item]?.displayName} size={20} />
                  <span className="drawer-row-name">{itemLabel(gamedata.items, item)}</span>
                  <span className="minibar">
                    <span className={band === "good" ? "" : band} style={{ width: `${Math.min(100, sat * 100)}%` }} />
                  </span>
                  <span className={`t-data-12 ${isProjected || selectedGroup.status === "planned" ? "projected" : ""}`}>
                    {fmtRate(rate)}
                    <span className="unit">/min</span>
                  </span>
                </div>
              );
            })}
            {gamedata.recipes[selectedGroup.recipe]?.products.map(([item]) => (
              <div className="drawer-row" key={item}>
                <ItemIcon item={item} displayName={gamedata.items[item]?.displayName} size={20} />
                <span className="drawer-row-name">→ {itemLabel(gamedata.items, item)}</span>
                <span className={`t-data-12 ${isProjected || selectedGroup.status === "planned" ? "projected" : ""}`}>
                  {item === POWER_ITEM ? (
                    fmtPower(dgSel?.outRates[item] ?? 0)
                  ) : (
                    <>
                      {fmtRate(dgSel?.outRates[item] ?? 0)}
                      <span className="unit">/min</span>
                    </>
                  )}
                </span>
              </div>
            ))}
          </section>

          {groupLogi && (
            <section className="insp-section" data-testid="insp-logistics">
              <h3 className="t-label">LOGISTICS</h3>
              <div className="drawer-row">
                <ItemIcon item={SPLITTER_CLASS} displayName="Splitter" size={20} />
                <span className="drawer-row-name">Splitters</span>
                <span className="t-data-12">
                  {groupLogi.logi.splitters.balanced}
                  <span className="insp-logi-alt mono"> · {groupLogi.logi.splitters.manifold} manifold</span>
                </span>
              </div>
              <div className="drawer-row">
                <ItemIcon item={MERGER_CLASS} displayName="Merger" size={20} />
                <span className="drawer-row-name">Mergers</span>
                <span className="t-data-12">
                  {groupLogi.logi.mergers.balanced}
                  <span className="insp-logi-alt mono"> · {groupLogi.logi.mergers.manifold} manifold</span>
                </span>
              </div>
              <div className="insp-note">
                Balanced 1→3 tree shown (even feed); manifold (one tap per machine) is the simpler
                alternative.
              </div>
              {groupLogi.tips.map((t, i) => (
                <div className="insp-logi-tip mono" key={i}>
                  ▸ {t}
                </div>
              ))}
              {selectedGroup.status === "planned" ? (
                // Materialize the bank into its real build: N individual machines
                // wired through actual splitter/merger junctions (undoable).
                <button
                  className="btn btn-primary insp-expand-btn"
                  onClick={() => {
                    void dispatch([{ type: "expand_group", id: selectedGroup.id }]);
                    setSelection(null); // the ×N group is replaced by the machines
                  }}
                  data-testid="btn-expand-bank"
                >
                  EXPAND INTO {effCount(selectedGroup)} MACHINES + SPLITTERS
                </button>
              ) : (
                <div className="insp-note">Expand is available on planned (◇) banks.</div>
              )}
            </section>
          )}

          <section className="insp-section">
            <h3 className="t-label">FEED BELTS</h3>
            {feedEdges.length === 0 && <div className="drawer-empty">No incoming belts.</div>}
            {feedEdges.map((e) => (
                <div className="drawer-row" key={e.id}>
                  <span className="drawer-row-name">{itemLabel(gamedata.items, e.item)}</span>
                  {/* efficiency grammar: only a solver-named BOTTLENECK belt
                      earns the upgrade nudge — a full belt meeting demand is
                      optimal, not an upgrade prompt */}
                  {bottlenecks.has(e.id) && e.status !== "built" && <span className="chip crit">UPGRADE?</span>}
                  {e.status === "built" ? (
                    <span className="mono t-data-12" title="Imported as built — rebuild in-game to change its tier.">
                      {edgeTierLabel(e.item, e.tier)} · BUILT
                    </span>
                  ) : (
                    <select
                      className="mono"
                      style={{ height: 24 }}
                      value={clampEdgeTier(gamedata, e.item, e.tier)}
                      onChange={(ev) => void dispatch([{ type: "set_edge_tier", id: e.id, tier: Number(ev.target.value) }])}
                    >
                      {transportTiers(gamedata, e.item).map((t) => (
                        <option key={t} value={t}>
                          {edgeTierLabel(e.item, t)} — {transportCapacity(gamedata, e.item, t)}/min
                        </option>
                      ))}
                    </select>
                  )}
                </div>
            ))}
          </section>
        </>
      )}

      {/* ---- selected junction ---- */}
      {selectedJunction && (
        <section className="insp-section">
          <h3 className="t-label">
            {(gamedata.buildables?.[selectedJunction.buildable]?.displayName ?? selectedJunction.kind).toUpperCase()}
          </h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Floor</span>
            <div className="floor-stepper">
              <button
                className="insp-clock-btn mono"
                disabled={selectedJunction.floor === 0}
                onClick={() =>
                  void dispatch([
                    { type: "set_junction_floor", id: selectedJunction.id, floor: selectedJunction.floor - 1 },
                  ])
                }
              >
                −
              </button>
              <span className="t-data-12 floor-value">F{selectedJunction.floor}</span>
              <button
                className="insp-clock-btn mono"
                onClick={() =>
                  void dispatch([
                    { type: "set_junction_floor", id: selectedJunction.id, floor: selectedJunction.floor + 1 },
                  ])
                }
              >
                +
              </button>
            </div>
          </div>
          {Object.values(plan.edges)
            .filter(
              (e) =>
                (e.from.kind === "junction" && e.from.id === selectedJunction.id) ||
                (e.to.kind === "junction" && e.to.id === selectedJunction.id),
            )
            .map((e) => {
              const inbound = e.to.kind === "junction" && e.to.id === selectedJunction.id;
              return (
                <div className="drawer-row" key={e.id}>
                  <span className="drawer-row-name">
                    {inbound ? "←" : "→"} {itemLabel(gamedata.items, e.item)}
                  </span>
                  <span className={`t-data-12 ${isProjected ? "projected" : ""}`}>
                    {fmtRate(df?.edges[e.id]?.flow ?? 0)}
                    <span className="unit">/min</span>
                  </span>
                </div>
              );
            })}
          <div className="insp-note">
            Junctions never change totals — they split, merge, or buffer the flow the solver routes through them.
          </div>
        </section>
      )}

      {/* ---- selected belt/pipe ---- */}
      {selectedEdge && (
        <section className="insp-section">
          <h3 className="t-label">
            {(isFluidItem(gamedata, selectedEdge.item) ? "PIPE" : "BELT")} — {itemLabel(gamedata.items, selectedEdge.item).toUpperCase()}
          </h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Tier</span>
            {selectedEdge.status === "built" ? (
              <span className="mono t-data-12" data-testid="edge-tier-built">
                {edgeTierLabel(selectedEdge.item, selectedEdge.tier)} —{" "}
                {transportCapacity(gamedata, selectedEdge.item, selectedEdge.tier)}/min · BUILT
              </span>
            ) : (
              <select
                className="mono"
                style={{ height: 24 }}
                value={clampEdgeTier(gamedata, selectedEdge.item, selectedEdge.tier)}
                onChange={(ev) => void dispatch([{ type: "set_edge_tier", id: selectedEdge.id, tier: Number(ev.target.value) }])}
                data-testid="edge-tier-select"
              >
                {transportTiers(gamedata, selectedEdge.item).map((t) => (
                  <option key={t} value={t}>
                    {edgeTierLabel(selectedEdge.item, t)} — {transportCapacity(gamedata, selectedEdge.item, t)}/min
                  </option>
                ))}
              </select>
            )}
          </div>
          {selectedEdge.status === "built" && (
            <div className="insp-note" data-testid="edge-tier-built-note">
              Imported as built — this {isFluidItem(gamedata, selectedEdge.item) ? "pipe" : "belt"}'s tier is fixed to your save. Rebuild it at a higher tier in-game, then re-import to raise its capacity here.
            </div>
          )}
          <div className="drawer-row">
            <span className="drawer-row-name">Load</span>
            <span className={`t-data-12 ${isProjected ? "projected" : ""}`}>
              {fmtRate(df?.edges[selectedEdge.id]?.flow ?? 0)}
              <span className="unit">/min</span>
            </span>
          </div>
        </section>
      )}

      {/* ---- selected boundary port ---- */}
      {selectedPort && selectedPort.direction === "in" && (
        <section className="insp-section">
          <h3 className="t-label">INPUT — {itemLabel(gamedata.items, selectedPort.item).toUpperCase()}</h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Ceiling (from node claim)</span>
            <span className="t-data-12">
              {selectedPort.rateCeiling != null ? fmtRate(selectedPort.rateCeiling) : "—"}
              <span className="unit">/min</span>
            </span>
          </div>
          {selectedPort.boundRoute ? (
            <button
              className="btn btn-ghost insp-send-btn"
              onClick={() => setSelection({ kind: "route", id: selectedPort.boundRoute! })}
              data-testid="btn-view-route"
            >
              VIEW ROUTE →
            </button>
          ) : (
            <>
              <button
                className="btn btn-primary insp-send-btn"
                onClick={() => setReceivingInto(selectedPort.id)}
                data-testid="btn-receive-from-factory"
              >
                ← RECEIVE FROM ANOTHER FACTORY
              </button>
              <div className="insp-note">Currently supply-assumed. Pull it from a factory that produces it.</div>
            </>
          )}
        </section>
      )}

      {/* An OUT port can supply another factory's input. This is the in-graph
          entry to inter-factory routing (previously map-right-drag only). */}
      {selectedPort && selectedPort.direction === "out" && (
        <section className="insp-section">
          <h3 className="t-label">OUTPUT — {itemLabel(gamedata.items, selectedPort.item).toUpperCase()}</h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Rate</span>
            <span className="t-data-12">
              {fmtRate(selectedPort.rate)}
              <span className="unit">/min</span>
            </span>
          </div>
          {selectedPort.boundRoute ? (
            <button
              className="btn btn-ghost insp-send-btn"
              onClick={() => setSelection({ kind: "route", id: selectedPort.boundRoute! })}
              data-testid="btn-view-route"
            >
              VIEW ROUTE →
            </button>
          ) : (
            <button
              className="btn btn-primary insp-send-btn"
              onClick={() => setSendingFrom(selectedPort.id)}
              data-testid="btn-send-to-factory"
            >
              SEND TO ANOTHER FACTORY →
            </button>
          )}
          <div className="insp-note">Feeds this output into another factory's input. That factory can take several inputs.</div>
        </section>
      )}

      {sendingFrom && (
        <SendToFactory sourceFactory={factoryId} initialOutPort={sendingFrom} onClose={() => setSendingFrom(null)} />
      )}
      {receivingInto && (
        <ReceiveFromFactory targetFactory={factoryId} initialInPort={receivingInto} onClose={() => setReceivingInto(null)} />
      )}

      <footer className="insp-footer">
        Edits apply instantly to the plan. On a ◆ built bank they become ◇ deltas — visible in DIFF until built
        in-game.
      </footer>
    </aside>
  );
}
