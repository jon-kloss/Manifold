// Inspector (mock 4a, right, 360px). OUTPUT TARGET slider re-solves live:
// T0 (WASM) on drag frames rendering italic projections; T1 on release with a
// settle flash. The slider hard-stops at the binding constraint and names it.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore, solveChip } from "../state/store";
import { buildSnapshot, ensureT0, t0SetTarget } from "../solver/t0";
import { footprintFor, footprintArea } from "./footprints";
import { fmtClock, fmtPower, fmtRate, flowBand, bottleneckEdges } from "../lib/format";
import { beltCapacity, effClock, POWER_ITEM, type DerivedFactory, type Id } from "../state/types";
import ItemIcon from "../lib/ItemIcon";

const CLOCK_STEPS = [0.5, 0.75, 1.0, 1.5, 2.5];

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
  const setProjected = useStore((s) => s.setProjected);

  const factory = plan.factories[factoryId];
  const outPort = factory?.ports.map((id) => plan.ports[id]).find((p) => p?.direction === "out");
  const authoritative = derived.factories[factoryId];
  const ceiling = authoritative?.targetCeiling ?? null;

  // Slider state: value tracks canonical rate except mid-drag.
  const [dragValue, setDragValue] = useState<number | null>(null);
  const dragging = dragValue !== null;
  const rate = dragging ? dragValue : outPort?.rate ?? 0;
  const wasmReady = useRef(false);

  useEffect(() => {
    void ensureT0().then(() => {
      wasmReady.current = true;
    });
  }, []);

  const sliderMax = useMemo(() => {
    const base = ceiling ? ceiling.maxRate : Math.max(10, (outPort?.rate ?? 0) * 2);
    return Math.max(1, base);
  }, [ceiling, outPort?.rate]);

  const onDrag = useCallback(
    (v: number) => {
      if (!outPort) return;
      // hard stop AT the ceiling, with tick magnetism (within 1% snaps to it)
      let clamped = ceiling ? Math.min(v, ceiling.maxRate) : v;
      if (ceiling && clamped > ceiling.maxRate - sliderMax * 0.01) clamped = ceiling.maxRate;
      setDragValue(clamped);
      if (!wasmReady.current) return;
      const snapshot = buildSnapshot(useStore.getState().plan, useStore.getState().gamedata, factoryId);
      if (!snapshot) return;
      const result = t0SetTarget(snapshot, outPort.id, clamped);
      if (result) setProjected({ factoryId, result, targetRate: clamped });
    },
    [outPort, ceiling, factoryId, setProjected, sliderMax],
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
  const chip = solveChip(authoritative);
  const atCeiling = !!ceiling && rate >= ceiling.maxRate - 1e-6;

  const bindingText = useMemo(() => {
    if (!ceiling) return null;
    const b = ceiling.binding;
    const itemName = gamedata.items[b.item]?.displayName ?? b.item;
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

  return (
    <aside className="inspector" data-testid="inspector">
      {/* ---- OUTPUT TARGET (factory-level) ---- */}
      {outPort && (
        <section className="insp-section">
          <h3 className="t-label">OUTPUT TARGET — {gamedata.items[outPort.item]?.displayName?.toUpperCase()}</h3>
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
          </div>
          <div className="insp-slider-wrap">
            <input
              type="range"
              className="insp-slider"
              min={0}
              max={sliderMax}
              step={sliderMax / 200}
              value={rate}
              onChange={(e) => onDrag(Number(e.target.value))}
              onPointerUp={onRelease}
              onKeyUp={(e) => {
                if (e.key === "ArrowLeft" || e.key === "ArrowRight") onRelease();
              }}
              data-testid="target-slider"
            />
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
              const cap = feeding.reduce((acc, e) => acc + beltCapacity(e.tier), 0);
              const sat = cap > 0 ? rate / cap : 0;
              const band = flowBand(sat, rate, feeding.some((e) => bottlenecks.has(e.id)));
              return (
                <div className="drawer-row" key={item}>
                  <ItemIcon item={item} displayName={gamedata.items[item]?.displayName} size={20} />
                  <span className="drawer-row-name">{gamedata.items[item]?.displayName ?? item}</span>
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
                <span className="drawer-row-name">→ {gamedata.items[item]?.displayName ?? item}</span>
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

          <section className="insp-section">
            <h3 className="t-label">FEED BELTS</h3>
            {feedEdges.length === 0 && <div className="drawer-empty">No incoming belts.</div>}
            {feedEdges.map((e) => (
                <div className="drawer-row" key={e.id}>
                  <span className="drawer-row-name">{gamedata.items[e.item]?.displayName ?? e.item}</span>
                  {/* efficiency grammar: only a solver-named BOTTLENECK belt
                      earns the upgrade nudge — a full belt meeting demand is
                      optimal, not an upgrade prompt */}
                  {bottlenecks.has(e.id) && <span className="chip crit">UPGRADE?</span>}
                  <select
                    className="mono"
                    style={{ height: 24 }}
                    value={e.tier}
                    onChange={(ev) => void dispatch([{ type: "set_edge_tier", id: e.id, tier: Number(ev.target.value) }])}
                  >
                    {[1, 2, 3, 4, 5, 6].map((t) => (
                      <option key={t} value={t}>
                        MK.{t} — {beltCapacity(t)}/min
                      </option>
                    ))}
                  </select>
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
                    {inbound ? "←" : "→"} {gamedata.items[e.item]?.displayName ?? e.item}
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

      {/* ---- selected belt ---- */}
      {selectedEdge && (
        <section className="insp-section">
          <h3 className="t-label">BELT — {gamedata.items[selectedEdge.item]?.displayName?.toUpperCase()}</h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Tier</span>
            <select
              className="mono"
              style={{ height: 24 }}
              value={selectedEdge.tier}
              onChange={(ev) => void dispatch([{ type: "set_edge_tier", id: selectedEdge.id, tier: Number(ev.target.value) }])}
              data-testid="edge-tier-select"
            >
              {[1, 2, 3, 4, 5, 6].map((t) => (
                <option key={t} value={t}>
                  MK.{t} — {beltCapacity(t)}/min
                </option>
              ))}
            </select>
          </div>
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
          <h3 className="t-label">INPUT — {gamedata.items[selectedPort.item]?.displayName?.toUpperCase()}</h3>
          <div className="drawer-row">
            <span className="drawer-row-name">Ceiling (from node claim)</span>
            <span className="t-data-12">
              {selectedPort.rateCeiling != null ? fmtRate(selectedPort.rateCeiling) : "—"}
              <span className="unit">/min</span>
            </span>
          </div>
        </section>
      )}

      <footer className="insp-footer">
        Edits apply instantly to the plan. On a ◆ built bank they become ◇ deltas — visible in DIFF until built
        in-game.
      </footer>
    </aside>
  );
}
