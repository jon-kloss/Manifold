// Status bar (24px): power draw, ◈ under-construction count, ⚠ BOTTLENECK
// belts (clickable), right-side totals. Counts collapse to a ⋯ chip in
// overlay mode.

import { useEffect, useMemo, useState } from "react";
import { useStore } from "../state/store";
import { fmtPower, bottleneckEdges, circuitHeadroom, powerLevel, type PowerLevel } from "../lib/format";

export default function StatusBar({ overlayMode }: { overlayMode: boolean }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const setView = useStore((s) => s.setView);
  const setSelection = useStore((s) => s.setSelection);
  const setDashboardOpen = useStore((s) => s.setDashboardOpen);
  const setAdvisorOpen = useStore((s) => s.setAdvisorOpen);
  const setAdvisorTab = useStore((s) => s.setAdvisorTab);
  const cmdError = useStore((s) => s.cmdError);
  const clearCmdError = useStore((s) => s.clearCmdError);
  const [expanded, setExpanded] = useState(false);

  // PR 3: the always-visible hook into the docked NEXT feed — the #1 move's
  // title, truncated. Hidden when there are no moves or the rank is still null
  // (honest quiet, no flash). Click deep-links to the advisor NEXT tab.
  const topMove = useStore((s) => s.rank?.opportunities[0] ?? null);

  // Build-queue resume affordance: Done/total chip that reopens the dashboard.
  const buildQueue = derived.buildQueue;
  const buildDone = useMemo(() => buildQueue.filter((s) => s.done).length, [buildQueue]);

  // auto-clear after ~6s; the `at` handshake keeps a stale timer from
  // dismissing an error that arrived after the timer was armed.
  useEffect(() => {
    if (!cmdError) return;
    const at = cmdError.at;
    const t = window.setTimeout(() => clearCmdError(at), 6000);
    return () => window.clearTimeout(t);
  }, [cmdError, clearCmdError]);

  const ucCount = useMemo(
    () =>
      Object.values(plan.groups).filter((g) => g.status === "under_construction").length +
      Object.values(plan.factories).filter((f) => f.status === "under_construction").length,
    [plan],
  );

  // Efficiency grammar: the alarm counts BOTTLENECK belts (solver-named
  // capacity bindings) — a merely full belt meeting demand is optimal.
  const critEdges = useMemo(() => {
    const out: { factory: string; edge: string }[] = [];
    for (const [fid, df] of Object.entries(derived.factories)) {
      for (const eid of bottleneckEdges(df)) out.push({ factory: fid, edge: eid });
    }
    return out;
  }, [derived]);

  // PWR chip: the bar's fill WIDTH and fill COLOR come from the SAME source —
  // the worst per-circuit headroom (orange is a verb — it follows the derived
  // condition). They previously used different denominators (aggregate
  // draw/generation for width, worst circuit for color), so an unbalanced
  // multi-grid empire rendered a nearly-empty bar painted alarm-red (or a full
  // green bar while one grid sat at the brink). No grids yet ⇒ both fall back
  // to the aggregate draw against total generation, so an ungridded empire's
  // overdraw still tints.
  const power = useMemo((): { level: PowerLevel; fillPct: number; title?: string } => {
    if (derived.circuits.length === 0) {
      const fillPct =
        derived.totalGenerationMw > 0
          ? (derived.totalPowerMw / derived.totalGenerationMw) * 100
          : derived.totalPowerMw / 5;
      return { level: powerLevel(circuitHeadroom(derived.totalGenerationMw, derived.totalPowerMw)), fillPct };
    }
    let worst = derived.circuits[0];
    let worstHeadroom = circuitHeadroom(worst.generationMw, worst.demandMw);
    for (const c of derived.circuits) {
      const h = circuitHeadroom(c.generationMw, c.demandMw);
      if (h < worstHeadroom) {
        worst = c;
        worstHeadroom = h;
      }
    }
    // A consumer-only grid (no generator in the component) is DEFINITIVELY
    // 100% overdrawn the moment it draws anything — render it full, matching
    // the audit drawer's card for the same circuit, never a near-empty
    // alarm-red sliver (the exact width/color contradiction this memo fixes).
    const fillPct =
      worst.generationMw > 0 ? (worst.demandMw / worst.generationMw) * 100 : worst.demandMw > 0 ? 100 : 0;
    return {
      level: powerLevel(worstHeadroom),
      fillPct,
      title: `Tightest grid — ${worst.name}: ${fmtPower(worst.demandMw)} of ${fmtPower(worst.generationMw)} generated`,
    };
  }, [derived.circuits, derived.totalGenerationMw, derived.totalPowerMw]);
  const powerClass = power.level === "crit" ? "sb-crit" : power.level === "warn" ? "sb-warn" : "";

  const jumpToCrit = () => {
    const first = critEdges[0];
    if (!first) return;
    setView({ mode: "factory", factoryId: first.factory });
    setSelection({ kind: "edge", id: first.edge });
  };

  const counts = (
    <>
      <span className="sb-item mono" title="Planned machines under construction">
        ◈ {ucCount}
      </span>
      <button
        className={`sb-item mono ${critEdges.length ? "sb-crit" : ""}`}
        data-testid="sb-bottleneck"
        onClick={jumpToCrit}
        disabled={!critEdges.length}
        title="Bottleneck belts — capacity caps demanded throughput"
      >
        ⚠ {critEdges.length}
      </button>
    </>
  );

  return (
    <footer className="statusbar">
      <span className={`sb-item mono ${powerClass}`} data-testid="sb-power" title={power.title}>
        PWR {fmtPower(derived.totalPowerMw)}
        {derived.totalGenerationMw > 0 && <span className="sb-gen"> / {fmtPower(derived.totalGenerationMw)}</span>}
        <span className="sb-powerbar" aria-hidden>
          <span
            className={power.level === "ok" ? "" : power.level}
            style={{ width: `${Math.min(100, power.fillPct)}%` }}
          />
        </span>
      </span>
      {overlayMode ? (
        <span style={{ position: "relative" }}>
          <button className="sb-item mono" onClick={() => setExpanded((e) => !e)}>
            ⋯
          </button>
          {expanded && <span className="sb-popover">{counts}</span>}
        </span>
      ) : (
        counts
      )}
      {buildQueue.length > 0 && (
        <button
          className="sb-item mono"
          data-testid="sb-resume"
          onClick={() => setDashboardOpen(true)}
          title="Resume — build queue (H)"
        >
          ▶ RESUME {buildDone}/{buildQueue.length}
        </button>
      )}
      {topMove && (
        <button
          className="sb-item mono sb-next"
          data-testid="sb-next"
          onClick={() => {
            setAdvisorOpen(true);
            setAdvisorTab("next");
          }}
          title={`Next move: ${topMove.title}`}
        >
          ▶ NEXT: <span className="sb-next-title">{topMove.title}</span>
        </button>
      )}
      {cmdError && (
        <button
          className="sb-item mono sb-error"
          data-testid="sb-error"
          onClick={() => clearCmdError(cmdError.at)}
          title={cmdError.message}
        >
          ⚠ <span className="sb-error-msg">{cmdError.message}</span>
        </button>
      )}
      <span className="sb-spring" />
      <span className="sb-item mono">
        {Object.keys(plan.factories).length} FACTORIES · {Object.keys(plan.groups).length} GROUPS ·{" "}
        {Object.keys(plan.nodeClaims).length} CLAIMS
      </span>
    </footer>
  );
}
