// Supply-chain wizard (mocks 5a/5b): 880px corner-cut modal over the dimmed
// map. Step 1 = goal sentence + constraints; step 2 = live phase list + real
// solver log (cancellable); step 3 hands off to the proposal review surface.
// Infeasible ≠ dead end (5c): best achievable + binding + one-tap relaxations.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../state/store";
import { backend } from "../state/backend";
import { fmtDuration, fmtRate, itemLabel } from "../lib/format";
import ItemCombobox from "../lib/ItemCombobox";
import type { WizardConstraints, WizardGoal, WizardInfeasible, WizardLogLine } from "../state/types";
import "./wizard.css";

const PHASES = ["DEMAND GRAPH", "RECIPE SELECTION", "SITING", "ROUTING"] as const;

const DEFAULT_CONSTRAINTS: WizardConstraints = {
  surplusFirst: true,
  maxNewSites: 2,
  nodeBudget: 3,
  purityFloor: "impure",
  powerMarginCap: 0.05,
  expandPreference: 0.5,
  includeAlternates: false,
};

export default function WizardModal() {
  const wizard = useStore((s) => s.wizard);
  const gamedata = useStore((s) => s.gamedata);
  const unlocked = useStore((s) => s.unlocked);
  const derived = useStore((s) => s.derived);
  const dispatch = useStore((s) => s.dispatch);
  const setWizard = useStore((s) => s.setWizard);
  const setReviewing = useStore((s) => s.setReviewing);

  const [step, setStep] = useState<1 | 2>(1);
  const [item, setItem] = useState("");
  const [rate, setRate] = useState(8);
  // total-quantity goal mode (milestone): off by default so the rate-only flow
  // is visually unchanged; toggling on reveals the total input + time ladder.
  const [totalOn, setTotalOn] = useState(false);
  const [total, setTotal] = useState(2500);
  const [constraints, setConstraints] = useState<WizardConstraints>(DEFAULT_CONSTRAINTS);
  const [log, setLog] = useState<WizardLogLine[]>([]);
  const [infeasible, setInfeasible] = useState<WizardInfeasible | null>(null);
  const jobRef = useRef<string | null>(null);
  const logRef = useRef<HTMLDivElement>(null);

  // craftable items only (recipes exist, not power, not raw-ore-only). W2b:
  // unlocked alternates are first-class, so an item reachable only through an
  // unlocked alt recipe is offered too.
  const craftable = useMemo(
    () =>
      Object.values(gamedata.items)
        .filter((i) =>
          Object.values(gamedata.recipes).some(
            (r) =>
              (!r.alternate || unlocked.has(r.className)) &&
              r.producedIn.length > 0 &&
              r.products.some(([p]) => p === i.className),
          ),
        )
        .sort((a, b) => a.displayName.localeCompare(b.displayName)),
    [gamedata, unlocked],
  );

  useEffect(() => {
    if (!wizard.open) return;
    setStep(1);
    setLog([]);
    setInfeasible(null);
    setTotalOn(false);
    if (wizard.prefill) {
      setItem(wizard.prefill.item);
      setRate(wizard.prefill.rate);
    } else if (!item && craftable.length > 0) {
      setItem(craftable[0].className);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wizard]);

  const close = useCallback(() => {
    if (jobRef.current) void backend.wizardCancel(jobRef.current);
    jobRef.current = null;
    setWizard({ open: false });
  }, [setWizard]);

  const solve = useCallback(
    async (goalOverride?: WizardGoal) => {
      const goal: WizardGoal = goalOverride ?? {
        items: [[item, rate]],
        constraints,
        // total-quantity mode: carry the target through the solver; the plan
        // itself is still driven by `rate` (the solver never reads milestone).
        ...(totalOn && total > 0 ? { milestone: { item, total, rate } } : {}),
      };
      setStep(2);
      setLog([]);
      setInfeasible(null);
      const jobId = await backend.wizardSolve(goal);
      jobRef.current = jobId;
      let seen = 0;
      const poll = async () => {
        if (jobRef.current !== jobId) return; // cancelled / superseded
        const p = await backend.wizardProgress(jobId, seen);
        if (p.log.length) {
          seen += p.log.length;
          setLog((l) => [...l, ...p.log]);
          requestAnimationFrame(() => logRef.current?.scrollTo(0, 1e6));
        }
        if (!p.done || !p.outcome) {
          setTimeout(() => void poll(), 120);
          return;
        }
        jobRef.current = null;
        const outcome = p.outcome;
        if (outcome.outcome === "proposal") {
          // store the draft through the ordinary command path, then review it
          const created = await dispatch([{ type: "create_proposal", proposal: outcome.proposal }]);
          if (!created) {
            // refusal is in the status bar; back to the goal form, wizard open
            setStep(1);
            return;
          }
          setWizard({ open: false });
          if (created[0]) setReviewing(created[0]);
        } else if (outcome.outcome === "infeasible") {
          setInfeasible(outcome);
        } else {
          setStep(1);
        }
      };
      void poll();
    },
    [item, rate, total, totalOn, constraints, dispatch, setReviewing, setWizard],
  );

  // ⏎ solves from step 1; ESC closes. Capture phase on purpose — MapView's
  // bubble-phase Escape ordering depends on it — which means this runs BEFORE
  // the ItemCombobox's own onKeyDown. While the combobox list is open it owns
  // both keys (Escape dismisses the list, Enter picks the highlighted option),
  // so yield to it explicitly instead of moving off capture.
  useEffect(() => {
    if (!wizard.open) return;
    const inOpenCombo = (t: EventTarget | null) =>
      t instanceof HTMLElement && !!t.closest(".item-combo")?.querySelector(".item-combo-list");
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (inOpenCombo(e.target)) return;
        e.stopPropagation();
        close();
      } else if (e.key === "Enter" && step === 1 && item) {
        if (inOpenCombo(e.target)) return;
        void solve();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [wizard.open, step, item, solve, close]);

  // Built once per log change, not per phaseState call — the phase list asks
  // 4× per render at the 120ms poll cadence, and rebuilding the Set walks the
  // whole log each time. Hoisted above the early return: hooks must run in the
  // same order every render, open or not.
  const seenPhases = useMemo(() => new Set(log.map((l) => l.phase)), [log]);

  if (!wizard.open) return null;

  const phaseState = (phase: string): "done" | "active" | "pending" => {
    const idx = PHASES.indexOf(phase as (typeof PHASES)[number]);
    const lastPhase = log[log.length - 1]?.phase;
    if (lastPhase === phase && !infeasible) return "active";
    if (seenPhases.has(phase)) return "done";
    const lastIdx = PHASES.indexOf(lastPhase as (typeof PHASES)[number]);
    return idx < lastIdx ? "done" : "pending";
  };

  const deficitChips = derived.deficits.slice(0, 4);

  return (
    <div className="wizard-scrim" data-testid="wizard-modal">
      <div className="wizard-modal">
        <header className="wizard-head">
          <span className="wizard-stamp mono">WIZARD</span>
          <span className="t-title">PLAN A SUPPLY CHAIN</span>
          <div className="wizard-steps mono">
            {(["1 GOAL", "2 SOLVE", "3 REVIEW"] as const).map((label, i) => (
              <span key={label} className={`wizard-step ${step === i + 1 ? "active" : step > i + 1 ? "done" : ""}`}>
                {label}
              </span>
            ))}
          </div>
          <button className="drawer-close" onClick={close} aria-label="Close">
            ×
          </button>
        </header>

        {step === 1 && (
          <div className="wizard-body">
            <div className="wizard-goal-sentence">
              <span className="t-label">PRODUCE</span>
              <ItemCombobox items={craftable} value={item} onChange={setItem} testid="wizard-item" />
              <span className="t-label">AT</span>
              <input
                type="number"
                className="mono wizard-rate"
                min={0.1}
                step={0.5}
                value={rate}
                onChange={(e) => setRate(Number(e.target.value))}
                data-testid="wizard-rate"
              />
              <span className="t-label">/MIN EMPIRE-WIDE</span>
            </div>

            {/* total-quantity goal (milestone): the game hands out huge total
                goals ("2,500 Versatile Frameworks") with no plan. Toggle it on
                to see how long the chosen rate takes — and faster alternatives. */}
            <div className="wizard-total-row">
              <label className="wizard-total-toggle">
                <input
                  type="checkbox"
                  checked={totalOn}
                  onChange={(e) => setTotalOn(e.target.checked)}
                  data-testid="wizard-total-toggle"
                />
                <span className="t-label">TOTAL-QUANTITY GOAL (MILESTONE)</span>
              </label>
              {totalOn && (
                <div className="wizard-total-inputs">
                  <span className="t-label">NEED</span>
                  <input
                    type="number"
                    className="mono wizard-total"
                    min={1}
                    step={100}
                    value={total}
                    onChange={(e) => setTotal(Number(e.target.value))}
                    data-testid="wizard-total"
                  />
                  <span className="t-label">TOTAL</span>
                </div>
              )}
            </div>
            {totalOn && total > 0 && (
              <div className="wizard-ladder mono" data-testid="wizard-ladder">
                {[rate, rate * 2, rate * 4]
                  .filter((r) => r > 0 && isFinite(r))
                  .map((r, i) => (
                    <span key={i} className="wizard-ladder-rung">
                      at {fmtRate(r)}/min → {fmtDuration(total / r)}
                    </span>
                  ))}
              </div>
            )}

            {deficitChips.length > 0 && (
              <div className="wizard-quickfill">
                <span className="t-label" style={{ color: "var(--ink-500)" }}>
                  QUICK-FILL FROM LIVE STATE
                </span>
                {deficitChips.map((d) => (
                  <button
                    key={`${d.factory}-${d.port}`}
                    className="chip crit"
                    onClick={() => {
                      setItem(d.item);
                      setRate(Math.max(1, Math.ceil(d.needed - d.supplied)));
                    }}
                  >
                    {(itemLabel(gamedata.items, d.item)).toUpperCase()} −{fmtRate(d.needed - d.supplied)}
                  </button>
                ))}
              </div>
            )}

            <h3 className="t-label" style={{ marginTop: 18 }}>
              CONSTRAINTS
            </h3>
            <div className="wizard-constraints">
              <label className="wc-row">
                <span>Surplus first — consume existing overproduction</span>
                <input
                  type="checkbox"
                  checked={constraints.surplusFirst}
                  onChange={(e) => setConstraints({ ...constraints, surplusFirst: e.target.checked })}
                />
              </label>
              <label className="wc-row">
                <span>Node budget</span>
                <input
                  type="number"
                  className="mono"
                  min={0}
                  max={12}
                  value={constraints.nodeBudget}
                  onChange={(e) => setConstraints({ ...constraints, nodeBudget: Number(e.target.value) })}
                  data-testid="wizard-node-budget"
                />
              </label>
              <label className="wc-row">
                <span>Purity floor</span>
                <select
                  className="mono"
                  value={constraints.purityFloor}
                  onChange={(e) =>
                    setConstraints({ ...constraints, purityFloor: e.target.value as WizardConstraints["purityFloor"] })
                  }
                >
                  <option value="impure">IMPURE — any</option>
                  <option value="normal">NORMAL+</option>
                  <option value="pure">PURE ONLY</option>
                </select>
              </label>
              <label className="wc-row">
                <span>
                  Also consider LOCKED alternates (suggestion only) — {unlocked.size} unlocked from your save
                </span>
                <input
                  type="checkbox"
                  checked={constraints.includeAlternates}
                  onChange={(e) => setConstraints({ ...constraints, includeAlternates: e.target.checked })}
                />
              </label>
              <label className="wc-row">
                <span>Power margin to keep</span>
                <span className="mono">
                  <input
                    type="number"
                    className="mono"
                    min={0}
                    max={50}
                    value={Math.round(constraints.powerMarginCap * 100)}
                    onChange={(e) => setConstraints({ ...constraints, powerMarginCap: Number(e.target.value) / 100 })}
                  />
                  %
                </span>
              </label>
            </div>

            <footer className="wizard-foot">
              <button className="btn btn-primary" onClick={() => void solve()} data-testid="wizard-solve">
                SOLVE ⏎
              </button>
              <span className="mono wizard-foot-note">RUNS LOCALLY · TYPICALLY &lt;1s AT THIS SCALE</span>
              <span className="wizard-foot-note">Result is a proposal — nothing is applied until you review it.</span>
            </footer>
          </div>
        )}

        {step === 2 && (
          <div className="wizard-body">
            <div className="wizard-phases">
              {PHASES.map((p) => {
                const st = phaseState(p);
                return (
                  <div key={p} className={`wizard-phase mono ${st}`}>
                    {st === "done" ? "✓" : st === "active" ? "▸" : "·"} {p}
                  </div>
                );
              })}
            </div>
            <div className="wizard-log mono" ref={logRef} data-testid="wizard-log">
              {/* Render only the tail: a runaway or genuinely huge solve can
                  stream tens of thousands of lines, and mounting them all
                  freezes the page — the scrollback beyond this window carries
                  no decision the review step doesn't already surface. */}
              {log.length > 400 && (
                // .wl-phase reused deliberately: the elision marker wants the
                // same de-emphasized styling as phase tags, not a new class.
                <div className="wl-phase">
                  … {log.length - 400} earlier line{log.length - 400 === 1 ? "" : "s"}
                </div>
              )}
              {log.slice(-400).map((l, i) => (
                <div key={log.length - Math.min(log.length, 400) + i}>
                  <span className="wl-phase">{l.phase}</span> {l.line}
                </div>
              ))}
              {!infeasible && <div className="wl-cursor">▉</div>}
            </div>

            {infeasible && (
              <div className="wizard-infeasible" data-testid="wizard-infeasible">
                <div className="t-label" style={{ color: "var(--flow-warn)" }}>
                  INFEASIBLE — {infeasible.binding.toUpperCase()}
                </div>
                <div className="wizard-foot-note">
                  Best achievable: <span className="mono">{fmtRate(infeasible.bestRate)}/min</span>
                </div>
                <div className="wizard-relaxations">
                  {/* only when the binding is actually node-shaped — a recipe
                      cycle or non-convergence can't be fixed by claiming more */}
                  {infeasible.relaxations.some((r) => r.toLowerCase().includes("node")) && (
                    <button
                      className="chip warn"
                      onClick={() => {
                        const c = { ...constraints, nodeBudget: constraints.nodeBudget + 1 };
                        setConstraints(c);
                        void solve({ items: [[item, rate]], constraints: c });
                      }}
                    >
                      +1 NODE CLAIM → RE-SOLVE
                    </button>
                  )}
                  {constraints.purityFloor !== "impure" && (
                    <button
                      className="chip warn"
                      onClick={() => {
                        const c = { ...constraints, purityFloor: "impure" as const };
                        setConstraints(c);
                        void solve({ items: [[item, rate]], constraints: c });
                      }}
                    >
                      DROP PURITY FLOOR → RE-SOLVE
                    </button>
                  )}
                  {!constraints.includeAlternates &&
                    infeasible.relaxations.some((r) => r.toLowerCase().includes("alternate")) && (
                      <button
                        className="chip warn"
                        onClick={() => {
                          const c = { ...constraints, includeAlternates: true };
                          setConstraints(c);
                          void solve({ items: [[item, rate]], constraints: c });
                        }}
                        data-testid="wizard-enable-alternates"
                      >
                        ENABLE ALTERNATES → RE-SOLVE
                      </button>
                    )}
                  {infeasible.bestRate > 0 && (
                    <button
                      className="chip"
                      onClick={() => {
                        const r = Math.floor(infeasible.bestRate * 10) / 10;
                        setRate(r);
                        void solve({ items: [[item, r]], constraints });
                      }}
                    >
                      ACCEPT {fmtRate(infeasible.bestRate)}/MIN
                    </button>
                  )}
                </div>
              </div>
            )}

            <footer className="wizard-foot">
              <button
                className="btn btn-ghost"
                onClick={() => {
                  if (jobRef.current) void backend.wizardCancel(jobRef.current);
                  jobRef.current = null;
                  setStep(1);
                }}
              >
                {infeasible ? "BACK TO GOAL" : "CANCEL"}
              </button>
            </footer>
          </div>
        )}
      </div>
    </div>
  );
}
