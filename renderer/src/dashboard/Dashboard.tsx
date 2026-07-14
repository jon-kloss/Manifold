// Session-resume dashboard (W1c): a dismissible overlay that auto-presents
// ONCE per plan-open when there is work to resume — what changed since the last
// import, what's half-built, what's starving, what's next, and progress. It is
// a pure projection over derived state (the build queue + deficits) and sits ON
// TOP of the restored map/factory view, revealing it unchanged on dismiss
// (Principle 1 — never replaces the map, never overrides the restored position).
//
// Each step row carries a checkbox → markBuildDone (one undoable SetBuildDone).
// Derived-Done steps render checked with an "in-game" provenance; manual
// overrides render with an OVERRIDE badge; route/claim steps are labelled
// manual-only because completion can't be detected in the save.

import { useEffect, useMemo, useState } from "react";
import { useStore } from "../state/store";
import { fmtRate } from "../lib/format";
import type { BuildStep, Cutover, CutoverPlan, CutoverStep } from "../state/types";
import "./dashboard.css";

const GLYPH: Record<string, string> = { pending: "◇", partial: "◈", done: "◆" };

const CUTOVER_PHASES = [
  { key: "build_new", label: "BUILD NEW" },
  { key: "switch", label: "SWITCH" },
  { key: "dismantle", label: "DISMANTLE" },
] as const;

export default function Dashboard() {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const lastImport = useStore((s) => s.lastImport);
  const setDashboardOpen = useStore((s) => s.setDashboardOpen);
  const setReviewing = useStore((s) => s.setReviewing);
  const setView = useStore((s) => s.setView);
  const setSelection = useStore((s) => s.setSelection);
  const setWizard = useStore((s) => s.setWizard);
  const markBuildDone = useStore((s) => s.markBuildDone);
  const cutoverPlan = useStore((s) => s.cutoverPlan);

  const itemName = (cls: string) => gamedata.items[cls]?.displayName ?? cls;
  const queue = derived.buildQueue;
  const cutovers = derived.cutovers;

  // Downtime is priced ON DEMAND (scratch-solved, ripple-inclusive) — fetch it
  // per open cutover when the dashboard is shown. Keyed by new-factory id; a
  // stable signature (ids + step done-flags) avoids a refetch loop as derived
  // state churns.
  const [downtimes, setDowntimes] = useState<Record<string, CutoverPlan>>({});
  const cutoverSig = useMemo(
    () =>
      cutovers
        .map((c) => `${c.newFactory}:${c.steps.map((s) => (s.done ? "1" : "0")).join("")}`)
        .join(","),
    [cutovers],
  );
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const out: Record<string, CutoverPlan> = {};
      for (const c of cutovers) {
        const p = await cutoverPlan(c.newFactory);
        if (p) out[c.newFactory] = p;
      }
      if (!cancelled) setDowntimes(out);
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cutoverSig, cutoverPlan]);

  const doneCount = useMemo(() => queue.filter((s) => s.done).length, [queue]);
  const partial = useMemo(() => queue.filter((s) => s.state === "partial"), [queue]);
  const nextStep = useMemo(() => queue.find((s) => !s.done), [queue]);
  const progress = useMemo(() => queue.find((s) => s.progress)?.progress ?? null, [queue]);
  const conflicts = useMemo(
    () => Object.entries(derived.nodes).filter(([, n]) => n.conflict),
    [derived.nodes],
  );
  const reimport = useMemo(
    () =>
      Object.values(plan.proposals).find(
        (p) => p.source === "save_reimport" && (p.status === "draft" || p.status === "reviewing"),
      ),
    [plan.proposals],
  );

  const dismiss = () => setDashboardOpen(false);

  // "go there" — reveal the step on the underlying view, then dismiss so the
  // restored map/factory shows through unchanged.
  const goTo = (step: BuildStep) => {
    if (step.kind === "route") {
      setView({ mode: "map" });
      setSelection({ kind: "route", id: step.id });
    } else if (step.factory) {
      setView({ mode: "factory", factoryId: step.factory });
      if (step.kind === "group") setSelection({ kind: "group", id: step.id });
    }
    dismiss();
  };

  // Toggle a step's completion. Sending `null` when the toggle lands back on
  // the derived answer keeps the override overlay sparse (auto-dissolve).
  const toggle = (step: BuildStep) => {
    const target = !step.done;
    const derivedDone = step.state === "done";
    void markBuildDone(step.id, target === derivedDone ? null : target);
  };

  const provenance = (step: BuildStep) => {
    if (step.overridden)
      return (
        <span className="dash-badge override" data-testid="step-override">
          OVERRIDE
        </span>
      );
    if (step.done) return <span className="dash-badge ingame">IN-GAME</span>;
    if (step.manualOnly) return <span className="dash-badge manual">MANUAL ONLY · can't detect in-game</span>;
    return null;
  };

  const stepRow = (step: BuildStep) => (
    <label className={`dash-step ${step.done ? "done" : ""}`} key={step.id} data-testid="build-step">
      <input type="checkbox" checked={step.done} onChange={() => toggle(step)} />
      <span className={`dash-glyph mono s-${step.state}`}>{GLYPH[step.state]}</span>
      <span className="dash-step-main">
        <span className="dash-step-label">{step.label}</span>
        <span className="dash-step-detail mono">{step.detail}</span>
      </span>
      {provenance(step)}
      <button
        className="chip dash-goto"
        onClick={(e) => {
          e.preventDefault();
          goTo(step);
        }}
      >
        GO THERE
      </button>
    </label>
  );

  // One cutover step row (reuses the ◇◈◆ grammar). Switch steps are manual-only;
  // BuildNew/Dismantle derive completion but can still be hand-overridden.
  const cutoverStepRow = (step: CutoverStep) => {
    const toggle = () => {
      const target = !step.done;
      const derivedDone = step.state === "done";
      void markBuildDone(step.id, target === derivedDone ? null : target);
    };
    const goToStep = () => {
      if (step.factory) {
        setView({ mode: "map" });
        setSelection({ kind: "factory", id: step.factory });
      }
      dismiss();
    };
    return (
      <label className={`dash-step ${step.done ? "done" : ""}`} key={step.id} data-testid="cutover-step">
        <input type="checkbox" checked={step.done} onChange={toggle} />
        <span className={`dash-glyph mono s-${step.state}`}>{GLYPH[step.state]}</span>
        <span className="dash-step-main">
          <span className="dash-step-label">{step.label}</span>
          <span className="dash-step-detail mono">{step.detail}</span>
        </span>
        {step.overridden ? (
          <span className="dash-badge override" data-testid="cutover-override">
            OVERRIDE
          </span>
        ) : step.done ? (
          <span className="dash-badge ingame">DONE</span>
        ) : step.manualOnly ? (
          <span className="dash-badge manual">MANUAL ONLY · repoint the belts</span>
        ) : null}
        {step.factory && (
          <button
            className="chip dash-goto"
            onClick={(e) => {
              e.preventDefault();
              goToStep();
            }}
          >
            GO THERE
          </button>
        )}
      </label>
    );
  };

  const cutoverCard = (c: Cutover) => {
    const dt = downtimes[c.newFactory];
    return (
      <div className="dash-cutover" key={c.newFactory} data-testid="cutover-card">
        <div className="dash-line">
          <span className="dash-step-label">
            {c.newName.toUpperCase()} <span className="mono dim">REPLACES</span> {c.oldName.toUpperCase()}
          </span>
        </div>
        {(c.nodeReuse || dt?.hard) && (
          <div className="dash-line crit-row" data-testid="cutover-node-reuse">
            <span className="mono crit">NODE REUSE — UNAVOIDABLE DOWNTIME</span>
          </div>
        )}
        {CUTOVER_PHASES.map(({ key, label }) => {
          const steps = c.steps.filter((s) => s.phase === key);
          if (steps.length === 0) return null;
          return (
            <div className="dash-cutover-phase" key={key}>
              <h4 className="t-label" data-testid={`cutover-phase-${key}`}>
                {label}
              </h4>
              {steps.map(cutoverStepRow)}
            </div>
          );
        })}
        {dt && dt.downtimeAvailable === false && (
          // Honest: the baseline couldn't be computed (old factory declares
          // output but doesn't produce in the solve). Informational, not an
          // error — ink, not a crit/warn colour. A silent-empty downtime here
          // would read as "no impact", which is exactly the dishonesty we avoid.
          <div className="dash-line mono info" data-testid="downtime-unavailable">
            DOWNTIME UNAVAILABLE — {dt.unavailableReason}
          </div>
        )}
        {dt && dt.downtimeAvailable && dt.dips.length > 0 && (
          <div className="dash-line" data-testid="cutover-downtime">
            {dt.dips.map((d, i) => {
              // A phase-2 (Dismantle) dip carries est_hours: 0 — the old output
              // never returns, so it's a PERMANENT SHORTFALL, not a downtime
              // window. Phase-1 (Switch) keeps the "~Nh (est)" build-window label.
              const permanent = d.phase === 2 || d.estHours === 0;
              return (
                <span className="chip warn" key={i} data-testid="downtime-dip">
                  {itemName(d.item).toUpperCase()} → {fmtRate(d.rate)}/min (was {fmtRate(d.baseline)}) ·{" "}
                  {permanent
                    ? "PERMANENT SHORTFALL"
                    : `~${d.estHours < 1 ? d.estHours.toFixed(1) : Math.round(d.estHours)}h (est)`}
                </span>
              );
            })}
          </div>
        )}
        {dt && dt.downtimeAvailable && dt.dips.length === 0 && (
          <div className="dash-line mono dim" data-testid="downtime-none">
            no production impact
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="dash-scrim" onClick={dismiss} data-testid="dashboard">
      <aside className="dash-panel" onClick={(e) => e.stopPropagation()}>
        <header className="dash-head">
          <span className="t-title">RESUME · {plan.meta.name || "PLAN"}</span>
          <span className="mono dash-hint">H TO REOPEN · ESC TO DISMISS</span>
          <button className="drawer-close" onClick={dismiss} aria-label="Dismiss">
            ×
          </button>
        </header>

        <div className="dash-body">
          {/* what changed since last import */}
          <section className="dash-section">
            <h3 className="t-label">WHAT CHANGED SINCE LAST IMPORT</h3>
            {lastImport ? (
              <div className="dash-line mono" data-testid="dash-last-import">
                {lastImport.saveName} · {lastImport.outcome.replace("_", " ").toUpperCase()} ·{" "}
                {lastImport.factoriesAdded > 0 && `+${lastImport.factoriesAdded} factories · `}
                {lastImport.groupsChanged > 0 && `${lastImport.groupsChanged} groups · `}
                {lastImport.at.slice(0, 16).replace("T", " ")}
              </div>
            ) : (
              <div className="dash-line mono dim">No save imported yet.</div>
            )}
            {reimport && (
              <div className="dash-line">
                <span className="mono warn">{reimport.title} — unreviewed drift</span>
                <button
                  className="chip warn"
                  data-testid="dash-review"
                  onClick={() => {
                    setReviewing(reimport.id);
                    dismiss();
                  }}
                >
                  REVIEW
                </button>
              </div>
            )}
          </section>

          {/* progress */}
          <section className="dash-section">
            <h3 className="t-label">PROGRESS</h3>
            <div className="dash-line mono" data-testid="dash-progress">
              {doneCount} / {queue.length} steps done
            </div>
            {progress && (
              <div className="dash-milestone" data-testid="dash-milestone">
                <div className="dash-line mono">
                  {Math.round(progress.built).toLocaleString("en-US")} /{" "}
                  {progress.total.toLocaleString("en-US")} {itemName(progress.item).toUpperCase()} built
                </div>
                <div className="dash-bar">
                  <span
                    style={{ width: `${Math.min(100, (progress.built / Math.max(1, progress.total)) * 100)}%` }}
                  />
                </div>
              </div>
            )}
          </section>

          {/* what's next */}
          {nextStep && (
            <section className="dash-section">
              <h3 className="t-label">WHAT'S NEXT</h3>
              {stepRow(nextStep)}
            </section>
          )}

          {/* half-built */}
          {partial.length > 0 && (
            <section className="dash-section">
              <h3 className="t-label">HALF-BUILT ({partial.length})</h3>
              {partial.map(stepRow)}
            </section>
          )}

          {/* starving */}
          {(derived.deficits.length > 0 || conflicts.length > 0) && (
            <section className="dash-section">
              <h3 className="t-label">STARVING ({derived.deficits.length + conflicts.length})</h3>
              {derived.deficits.map((d) => (
                <div className="dash-line crit-row" key={`${d.factory}-${d.port}`}>
                  <span className="dash-step-label">
                    {plan.factories[d.factory]?.name ?? "?"} starved of {itemName(d.item)}
                  </span>
                  <span className="mono crit">−{fmtRate(d.needed - d.supplied)}/min</span>
                  <button
                    className="chip warn"
                    onClick={() =>
                      setWizard({ open: true, prefill: { item: d.item, rate: Math.ceil(d.needed - d.supplied) } })
                    }
                  >
                    FIX WITH SOLVER
                  </button>
                  <button
                    className="chip"
                    onClick={() => {
                      if (d.route) {
                        setView({ mode: "map" });
                        setSelection({ kind: "route", id: d.route });
                        dismiss();
                      }
                    }}
                  >
                    TRACE
                  </button>
                </div>
              ))}
              {conflicts.map(([nodeId, n]) => (
                <div className="dash-line crit-row" key={nodeId}>
                  <span className="dash-step-label">Node {nodeId} double-booked</span>
                  <span className="mono crit">⚠×{n.claims}</span>
                  <button
                    className="chip"
                    onClick={() => {
                      setView({ mode: "map" });
                      setSelection({ kind: "node", id: nodeId });
                      dismiss();
                    }}
                  >
                    TRACE
                  </button>
                </div>
              ))}
            </section>
          )}

          {/* cutover timeline (W2a) — refactor/replacement plans, phased */}
          {cutovers.length > 0 && (
            <section className="dash-section" data-testid="cutover-timeline">
              <h3 className="t-label">CUTOVER TIMELINE ({cutovers.length})</h3>
              {cutovers.map(cutoverCard)}
            </section>
          )}

          {/* the whole queue */}
          <section className="dash-section">
            <h3 className="t-label">BUILD QUEUE ({queue.length})</h3>
            {queue.length === 0 ? (
              <div className="dash-line mono dim">Nothing planned — the build queue is clear.</div>
            ) : (
              queue.map(stepRow)
            )}
          </section>
        </div>
      </aside>
    </div>
  );
}
