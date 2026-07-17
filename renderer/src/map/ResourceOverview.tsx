// Always-on empire resource ledger (map screen, top-left). Aggregates every
// factory's solved group in/out rates + raw boundary supply into one make/use/
// net table so a player can see, at a glance, what the whole empire produces
// and consumes without opening any single factory.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { POWER_ITEM } from "../state/types";
import { fmtRate, fmtPower, itemLabel } from "../lib/format";
import ItemIcon from "../lib/ItemIcon";

interface Row {
  item: string;
  produced: number;
  consumed: number;
  net: number;
}

export default function ResourceOverview() {
  const derived = useStore((s) => s.derived);
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const [open, setOpen] = useState(true);

  const rows = useMemo<Row[]>(() => {
    const produced = new Map<string, number>();
    const consumed = new Map<string, number>();
    const add = (m: Map<string, number>, item: string, rate: number) => {
      if (!rate) return;
      m.set(item, (m.get(item) ?? 0) + rate);
    };
    for (const df of Object.values(derived.factories)) {
      for (const g of Object.values(df.groups)) {
        for (const [item, rate] of Object.entries(g.outRates)) {
          if (item !== POWER_ITEM) add(produced, item, rate);
        }
        for (const [item, rate] of Object.entries(g.inRates)) {
          if (item !== POWER_ITEM) add(consumed, item, rate);
        }
      }
    }
    // Raw feedstock: a boundary IN port that is NOT bound to an inter-factory
    // route is new supply entering the empire (a claimed node, or assumed
    // external supply) — count its realized rate as production so raw ores
    // don't read as a pure deficit. Route-bound in-ports are skipped: that item
    // was already counted at the group that produced it upstream.
    for (const p of Object.values(plan.ports)) {
      if (p.direction !== "in" || p.boundRoute) continue;
      const realized = derived.factories[p.factory]?.ports[p.id];
      if (realized) add(produced, p.item, realized);
    }

    const items = new Set([...produced.keys(), ...consumed.keys()]);
    const out: Row[] = [];
    for (const item of items) {
      const pr = produced.get(item) ?? 0;
      const co = consumed.get(item) ?? 0;
      if (pr < 1e-6 && co < 1e-6) continue;
      out.push({ item, produced: pr, consumed: co, net: pr - co });
    }
    // Busiest resources first — biggest total throughput at the top.
    out.sort((a, b) => b.produced + b.consumed - (a.produced + a.consumed));
    return out;
  }, [derived, plan.ports]);

  const genMw = derived.totalGenerationMw;
  const drawMw = derived.totalPowerMw;
  const powerNet = genMw - drawMw;

  return (
    <div className="resource-overview" data-testid="resource-overview">
      <button className="t-label ro-toggle" onClick={() => setOpen(!open)}>
        RESOURCES {open ? "▾" : "▸"}
      </button>
      {open && (
        <div className="ro-body">
          <div className="ro-power" title="Empire power: generation vs draw">
            <span className="ro-power-label">⚡ POWER</span>
            <span className="ro-cell ro-make">{fmtPower(genMw)}</span>
            <span className="ro-cell ro-use">{fmtPower(drawMw)}</span>
            <span className={`ro-cell ro-net ${powerNet < -0.01 ? "deficit" : "surplus"}`}>
              {powerNet >= 0 ? "+" : "−"}
              {fmtPower(Math.abs(powerNet))}
            </span>
          </div>
          {rows.length === 0 ? (
            <div className="ro-empty">No production yet — add a factory and set an output target.</div>
          ) : (
            <>
              <div className="ro-head mono">
                <span />
                <span className="ro-cell">MAKE</span>
                <span className="ro-cell">USE</span>
                <span className="ro-cell">NET</span>
              </div>
              <div className="ro-rows">
                {rows.map((r) => (
                  <div className="ro-row" key={r.item} title={itemLabel(gamedata.items, r.item)}>
                    <span className="ro-name">
                      <ItemIcon item={r.item} displayName={itemLabel(gamedata.items, r.item)} size={20} />
                      <span className="ro-name-text">{itemLabel(gamedata.items, r.item)}</span>
                    </span>
                    <span className="ro-cell ro-make">{r.produced > 1e-6 ? fmtRate(r.produced) : "·"}</span>
                    <span className="ro-cell ro-use">{r.consumed > 1e-6 ? fmtRate(r.consumed) : "·"}</span>
                    <span
                      className={`ro-cell ro-net ${r.net > 0.01 ? "surplus" : r.net < -0.01 ? "deficit" : "balanced"}`}
                    >
                      {Math.abs(r.net) < 0.01 ? "0" : `${r.net > 0 ? "+" : "−"}${fmtRate(Math.abs(r.net))}`}
                    </span>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
