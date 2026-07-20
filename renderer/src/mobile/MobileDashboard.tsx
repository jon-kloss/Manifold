// #110 — read-only mobile companion dashboard. At the phone breakpoint the
// editing surfaces (React Flow box-select/right-click, precise belt wiring,
// map editing) degrade badly on touch, so instead of shrinking them the app
// swaps to this full-screen, glanceable empire status board: power balance
// (gen/draw/net/headroom + per-grid), deficit/brownout alerts, and the
// resource make/use/net ledger with per-item factory drill-down. Zero writes —
// the one use case is checking empire status on a phone while building on the
// PC next to the game.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { circuitHeadroom, fmtPower, fmtRate, itemLabel, powerLevel } from "../lib/format";
import { buildLedgerRows } from "../lib/ledger";
import ItemIcon from "../lib/ItemIcon";
import "./mobile.css";

const LEVEL_WORD = { ok: "OK", warn: "TIGHT", crit: "BROWNOUT RISK" } as const;

const sign = (v: number): string => (v >= 0 ? "+" : "−");

/** Draw-vs-generation bar fill % — same convention as ResourceOverview:
 *  zero generation with real draw is a blackout, filled to 100 (red). */
const barFill = (gen: number, draw: number): number =>
  gen > 0 ? Math.min(100, (draw / gen) * 100) : draw > 0 ? 100 : 0;

export default function MobileDashboard() {
  const derived = useStore((s) => s.derived);
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const [expanded, setExpanded] = useState<string | null>(null);

  const rows = useMemo(
    () => buildLedgerRows(derived, plan, gamedata.items),
    [derived, plan, gamedata.items],
  );

  const genMw = derived.totalGenerationMw;
  const drawMw = derived.totalPowerMw;
  const powerNet = genMw - drawMw;
  const empireHeadroom = circuitHeadroom(genMw, drawMw);
  // One power verdict: the WORST of empire + every grid — a healthy empire
  // total must not mask one browning-out coastal grid.
  const worst = derived.circuits.reduce(
    (acc, c) => {
      const l = powerLevel(circuitHeadroom(c.generationMw, c.demandMw));
      return l === "crit" || acc === "crit" ? "crit" : l === "warn" ? "warn" : acc;
    },
    powerLevel(empireHeadroom),
  );

  const badCircuits = derived.circuits.filter(
    (c) => powerLevel(circuitHeadroom(c.generationMw, c.demandMw)) !== "ok",
  );
  const factories = Object.keys(plan.factories).length;

  return (
    <div className="mobile-dash" data-testid="mobile-dashboard">
      <header className="md-header">
        <span className="md-brand t-title">MANIFOLD</span>
        <span className="md-note mono">READ-ONLY · EDITING IS BEST ON DESKTOP</span>
      </header>

      {factories === 0 ? (
        <section className="md-card">
          <div className="md-empty mono">
            NO PLAN YET — this companion view is read-only. Open MANIFOLD on a
            desktop to start planning; your empire status will show here.
          </div>
        </section>
      ) : (
        <>
          <section className="md-card" data-testid="md-power">
            <div className="md-card-head">
              <h2 className="t-label">⚡ POWER</h2>
              <span className={`md-level ${worst}`}>{LEVEL_WORD[worst]}</span>
            </div>
            {genMw > 0 || drawMw > 0 ? (
              <>
                <div className="md-power-net">
                  <span className={`md-net-big ${powerNet < -0.01 ? "deficit" : "surplus"}`}>
                    {sign(powerNet)}
                    {fmtPower(Math.abs(powerNet))}
                  </span>
                  <span className="md-headroom mono">{Math.round(empireHeadroom * 100)}% HEADROOM</span>
                </div>
                <div className="md-genline mono">
                  GEN {fmtPower(genMw)} · DRAW {fmtPower(drawMw)}
                </div>
                <div className="md-bar">
                  <span className={worst} style={{ width: `${barFill(genMw, drawMw)}%` }} />
                </div>
              </>
            ) : (
              <div className="md-genline mono ghost">NO GENERATION · NO DRAW</div>
            )}
            {derived.circuits.map((c) => {
              const h = circuitHeadroom(c.generationMw, c.demandMw);
              const l = powerLevel(h);
              return (
                <div className="md-grid-row" key={c.name} data-testid="md-grid">
                  <span className={`md-dot ${l}`} />
                  <span className="md-grid-name mono">{c.name}</span>
                  <span className="md-grid-figs mono">
                    {fmtPower(c.demandMw)} / {fmtPower(c.generationMw)}
                  </span>
                </div>
              );
            })}
          </section>

          <section className="md-card" data-testid="md-alerts">
            <div className="md-card-head">
              <h2 className="t-label">▲ ALERTS</h2>
              {derived.deficits.length + badCircuits.length === 0 && (
                <span className="md-level ok">ALL CLEAR</span>
              )}
            </div>
            {badCircuits.map((c) => {
              const h = circuitHeadroom(c.generationMw, c.demandMw);
              return (
                <div className="md-alert mono" key={c.name}>
                  ⚡ {c.name} — {(h * 100).toFixed(1)}% HEADROOM
                  {c.nextShed ? ` · NEXT SHED ${c.nextShed}` : ""}
                </div>
              );
            })}
            {derived.deficits.map((d) => (
              <div className="md-alert mono" key={`${d.factory}:${d.port}`}>
                ▲ SHORT {fmtRate(d.needed - d.supplied)}/MIN {itemLabel(gamedata.items, d.item)} AT{" "}
                {plan.factories[d.factory]?.name ?? "?"}
              </div>
            ))}
          </section>

          <section className="md-card" data-testid="md-resources">
            <div className="md-card-head">
              <h2 className="t-label">RESOURCES</h2>
              <span className="md-note mono">{rows.length} ITEMS</span>
            </div>
            <div className="md-ledger-head mono">
              <span />
              <span>MAKE</span>
              <span>USE</span>
              <span>NET</span>
            </div>
            {rows.map((r) => (
              <div key={r.item}>
                <button
                  className="md-row"
                  data-testid="md-ledger-row"
                  onClick={() => setExpanded(expanded === r.item ? null : r.item)}
                >
                  <span className="md-row-name">
                    <ItemIcon item={r.item} displayName={r.label} size={20} />
                    <span className="md-row-label">{r.label}</span>
                    {r.raw && <span className="md-tag-raw">RAW</span>}
                  </span>
                  <span className="md-cell mono">{r.produced > 1e-6 ? fmtRate(r.produced) : "·"}</span>
                  <span className="md-cell mono">{r.consumed > 1e-6 ? fmtRate(r.consumed) : "·"}</span>
                  <span
                    className={`md-cell mono ${r.net > 0.01 ? "surplus" : r.net < -0.01 ? "deficit" : ""}`}
                  >
                    {Math.abs(r.net) < 0.01 ? "0" : `${sign(r.net)}${fmtRate(Math.abs(r.net))}`}
                  </span>
                </button>
                {expanded === r.item && (
                  <div className="md-drill" data-testid="md-drill">
                    {r.makers.length > 0 && (
                      <div className="md-drill-block">
                        <div className="md-drill-h mono">{r.raw ? "SUPPLIED BY" : "MADE BY"}</div>
                        {r.makers.map((m) => (
                          <div className="md-drill-row mono" key={m.factory}>
                            <span>{m.name}</span>
                            <span className="surplus">+{fmtRate(m.rate)}</span>
                          </div>
                        ))}
                      </div>
                    )}
                    {r.users.length > 0 && (
                      <div className="md-drill-block">
                        <div className="md-drill-h mono">CONSUMED BY</div>
                        {r.users.map((u) => (
                          <div className="md-drill-row mono" key={u.factory}>
                            <span>{u.name}</span>
                            <span className="deficit">−{fmtRate(u.rate)}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            ))}
            {rows.length === 0 && (
              <div className="md-empty mono">No solved production yet.</div>
            )}
          </section>
        </>
      )}

      <footer className="md-footer mono">
        {factories} {factories === 1 ? "FACTORY" : "FACTORIES"} · GLANCE VIEW — OPEN ON DESKTOP TO EDIT
      </footer>
    </div>
  );
}
