// BUILD SHEET panel (read-only export): the plan→game bridge. Renders one
// factory's derived spec — machines & clocks, inputs + honest source targets,
// outputs, routes + tiers, power — in the design language, with a COPY button
// (plain-text/markdown to the clipboard) and print-friendly CSS. Never mutates
// the plan; every number comes from the derived projection.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { fmtKm, fmtPower, fmtRate } from "../lib/format";
import type { Id } from "../state/types";
import { composeBuildSheet, sheetToText } from "./buildSheetModel";
import "./buildsheet.css";

export default function BuildSheet({ factoryId, onClose }: { factoryId: Id; onClose: () => void }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const world = useStore((s) => s.world);
  const [copied, setCopied] = useState(false);

  const sheet = useMemo(
    () => composeBuildSheet(factoryId, plan, derived, gamedata, world),
    [factoryId, plan, derived, gamedata, world],
  );

  if (!sheet) return null;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(sheetToText(sheet));
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="bs-scrim" data-testid="build-sheet" onClick={onClose}>
      <div className="bs-modal" onClick={(e) => e.stopPropagation()}>
        <header className="bs-head">
          <span className="bs-stamp mono">BUILD SHEET</span>
          <span className="t-title">{sheet.name.toUpperCase()}</span>
          <span className="mono bs-sub">
            {sheet.region.toUpperCase()} · {sheet.statusGlyph} {sheet.statusText} ·{" "}
            <span className="bs-power">{fmtPower(sheet.powerMw)}</span>
          </span>
          <div className="bs-actions">
            <button
              className="btn btn-primary bs-copy"
              onClick={() => void copy()}
              data-testid="btn-build-sheet-copy"
            >
              {copied ? "COPIED ✓" : "COPY"}
            </button>
            <button
              className="btn btn-ghost bs-print"
              onClick={() => window.print()}
              data-testid="btn-build-sheet-print"
            >
              PRINT
            </button>
            <button className="drawer-close" onClick={onClose} aria-label="Close">
              ×
            </button>
          </div>
        </header>

        <div className="bs-body" data-testid="build-sheet-body">
          <section className="bs-section" data-testid="bs-machines">
            <h3 className="t-label">MACHINES</h3>
            {sheet.machines.length === 0 && <div className="bs-empty">No machine groups.</div>}
            {sheet.machines.map((m, i) => (
              <div className="bs-row" key={i}>
                <span className="mono bs-count">{m.count}×</span>
                <span className="bs-name">{m.machine}</span>
                <span className="mono bs-clock">@ {m.clock}</span>
                <span className="bs-recipe">— {m.recipe}</span>
              </div>
            ))}
          </section>

          <section className="bs-section" data-testid="bs-inputs">
            <h3 className="t-label">INPUTS</h3>
            {sheet.inputs.length === 0 && <div className="bs-empty">No inputs.</div>}
            {sheet.inputs.map((p, i) => (
              <div className="bs-row" key={i}>
                <span className="bs-name">{p.item}</span>
                <span className="mono t-data-12 bs-rate">
                  {fmtRate(p.rate)}
                  <span className="unit">/min</span>
                </span>
                <span className="mono bs-src">{p.source}</span>
              </div>
            ))}
          </section>

          <section className="bs-section" data-testid="bs-outputs">
            <h3 className="t-label">OUTPUTS</h3>
            {sheet.outputs.length === 0 && <div className="bs-empty">No outputs.</div>}
            {sheet.outputs.map((p, i) => (
              <div className="bs-row" key={i}>
                <span className="bs-name">{p.item}</span>
                <span className="mono t-data-12 bs-rate">
                  {fmtRate(p.rate)}
                  <span className="unit">/min</span>
                </span>
                <span className="mono bs-src">{p.source}</span>
              </div>
            ))}
          </section>

          <section className="bs-section" data-testid="bs-routes">
            <h3 className="t-label">ROUTES</h3>
            {sheet.routes.length === 0 && <div className="bs-empty">No routes touch this factory.</div>}
            {sheet.routes.map((r, i) => (
              <div className="bs-row" key={i}>
                <span className="bs-name">
                  {r.item} <span className="bs-arrow">{r.dir === "out" ? "→" : "←"}</span> {r.other}
                </span>
                <span className="mono bs-tier">{r.tier}</span>
                <span className="mono t-data-12 bs-rate">
                  {fmtRate(r.rate)}
                  <span className="unit">/min</span>
                </span>
                {r.lengthM > 0 && <span className="mono bs-len">{fmtKm(r.lengthM)}</span>}
              </div>
            ))}
          </section>

          <section className="bs-section" data-testid="bs-power">
            <h3 className="t-label">POWER</h3>
            <div className="bs-row">
              <span className="bs-name">Total draw at planned clocks</span>
              <span className="mono t-data-12 bs-power">{fmtPower(sheet.powerMw)}</span>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}
