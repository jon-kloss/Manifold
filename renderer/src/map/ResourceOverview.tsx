// Empire resource + power sidebar (map screen, top-left). One surface, three
// states: a collapsed rail (a power dot + deficit count you can ignore until
// they turn red), a brief panel (headline power balance + the busiest
// resources), and a detailed panel (searchable ledger with per-item factory
// drill-down and per-grid power). Same aggregation as before: solved group
// in/out rates + raw boundary supply, POWER_ITEM excluded.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { POWER_ITEM } from "../state/types";
import type { DeficitRow, DerivedCircuit, Id } from "../state/types";
import { circuitHeadroom, fmtPower, fmtRate, itemLabel, powerLevel } from "../lib/format";
import { minBeltTier } from "../graph/logistics";
import ItemIcon from "../lib/ItemIcon";

type PanelState = "collapsed" | "brief" | "detailed";
type KindFilter = "all" | "raw" | "made";
type BalFilter = "all" | "surplus" | "deficit";

interface Contribution {
  factory: Id;
  name: string;
  rate: number;
}

interface Row {
  item: string;
  label: string;
  /** received raw boundary supply (a claimed node / assumed external feed) */
  raw: boolean;
  produced: number;
  consumed: number;
  net: number;
  makers: Contribution[];
  users: Contribution[];
}

/** How many rows the brief state shows — busiest first; the footer names the rest. */
const BRIEF_MAX = 8;

const LEVEL_WORD = { ok: "OK", warn: "TIGHT", crit: "BROWNOUT RISK" } as const;

const sign = (v: number): string => (v >= 0 ? "+" : "−");

/** Draw-vs-generation bar fill %. Zero generation with real draw is a blackout /
 *  over-capacity — the most critical state — so it fills to 100 (red), never 0
 *  (which would read as "nothing happening"). */
const barFill = (gen: number, draw: number): number =>
  gen > 0 ? Math.min(100, (draw / gen) * 100) : draw > 0 ? 100 : 0;

export default function ResourceOverview() {
  const derived = useStore((s) => s.derived);
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const setSelection = useStore((s) => s.setSelection);
  const requestFly = useStore((s) => s.requestFly);

  const [ui, setUi] = useState<PanelState>("brief");
  const [secPower, setSecPower] = useState(true);
  const [secRes, setSecRes] = useState(true);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<KindFilter>("all");
  const [bal, setBal] = useState<BalFilter>("all");
  const [expanded, setExpanded] = useState<string | null>(null);

  const rows = useMemo<Row[]>(() => {
    const produced = new Map<string, number>();
    const consumed = new Map<string, number>();
    // per-item, per-factory contributions — the drill-down's evidence
    const makers = new Map<string, Map<Id, number>>();
    const users = new Map<string, Map<Id, number>>();
    const raw = new Set<string>();
    const add = (m: Map<string, number>, item: string, rate: number) => {
      if (!rate) return;
      m.set(item, (m.get(item) ?? 0) + rate);
    };
    const bump = (m: Map<string, Map<Id, number>>, item: string, fid: Id, rate: number) => {
      if (rate < 1e-9) return;
      let per = m.get(item);
      if (!per) {
        per = new Map();
        m.set(item, per);
      }
      per.set(fid, (per.get(fid) ?? 0) + rate);
    };
    for (const [fid, df] of Object.entries(derived.factories)) {
      for (const g of Object.values(df.groups)) {
        for (const [item, rate] of Object.entries(g.outRates)) {
          if (item === POWER_ITEM) continue;
          add(produced, item, rate);
          bump(makers, item, fid, rate);
        }
        for (const [item, rate] of Object.entries(g.inRates)) {
          if (item === POWER_ITEM) continue;
          add(consumed, item, rate);
          bump(users, item, fid, rate);
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
      if (realized) {
        add(produced, p.item, realized);
        bump(makers, p.item, p.factory, realized);
        raw.add(p.item);
      }
    }
    const contribs = (m: Map<string, Map<Id, number>> | undefined, item: string): Contribution[] =>
      [...(m?.get(item)?.entries() ?? [])]
        .map(([factory, rate]) => ({ factory, name: plan.factories[factory]?.name ?? "?", rate }))
        .sort((a, b) => b.rate - a.rate);
    const items = new Set([...produced.keys(), ...consumed.keys()]);
    const out: Row[] = [];
    for (const item of items) {
      const pr = produced.get(item) ?? 0;
      const co = consumed.get(item) ?? 0;
      if (pr < 1e-6 && co < 1e-6) continue;
      out.push({
        item,
        label: itemLabel(gamedata.items, item),
        raw: raw.has(item),
        produced: pr,
        consumed: co,
        net: pr - co,
        makers: contribs(makers, item),
        users: contribs(users, item),
      });
    }
    // Busiest resources first — biggest total throughput at the top.
    out.sort((a, b) => b.produced + b.consumed - (a.produced + a.consumed));
    return out;
  }, [derived, plan.ports, plan.factories, gamedata.items]);

  const deficitsByItem = useMemo(() => {
    const m = new Map<string, DeficitRow[]>();
    for (const d of derived.deficits) {
      const list = m.get(d.item);
      if (list) list.push(d);
      else m.set(d.item, [d]);
    }
    return m;
  }, [derived.deficits]);

  const genMw = derived.totalGenerationMw;
  const drawMw = derived.totalPowerMw;
  const powerNet = genMw - drawMw;
  const empireHeadroom = circuitHeadroom(genMw, drawMw);
  // The rail's one signal: the WORST of empire + every grid — a healthy empire
  // total must not mask one browning-out coastal grid.
  const worst = derived.circuits.reduce(
    (acc, c) => {
      const l = powerLevel(circuitHeadroom(c.generationMw, c.demandMw));
      return l === "crit" || acc === "crit" ? "crit" : l === "warn" ? "warn" : acc;
    },
    powerLevel(empireHeadroom),
  );
  const deficitCount = derived.deficits.length;

  const problems: string[] = [];
  if (deficitCount > 0) problems.push(`${deficitCount} SUPPLY DEFICIT${deficitCount === 1 ? "" : "S"}`);
  for (const c of derived.circuits) {
    const h = circuitHeadroom(c.generationMw, c.demandMw);
    if (powerLevel(h) !== "ok") problems.push(`${c.name} ${(h * 100).toFixed(1)}% HEADROOM`);
  }

  const q = query.trim().toLowerCase();
  const shown = rows.filter(
    (r) =>
      (!q || r.label.toLowerCase().includes(q)) &&
      (kind === "all" || (kind === "raw" ? r.raw : !r.raw)) &&
      // "deficit" = short SOMEWHERE: empire net-negative OR a per-port shortfall
      // (derived.deficits), so it matches the items the problem strip counts —
      // an item made in surplus empire-wide but starved at one routed factory
      // still shows here.
      (bal === "all" ? true : bal === "surplus" ? r.net > 0.01 : r.net < -0.01 || deficitsByItem.has(r.item)),
  );

  const stepUp = () => setUi(ui === "collapsed" ? "brief" : "detailed");
  const stepDown = () => {
    setUi(ui === "detailed" ? "brief" : "collapsed");
    setExpanded(null);
  };
  const goProblems = () => {
    setUi("detailed");
    // Force both accordions open so the problem the strip names is actually
    // visible; only apply the deficit filter when there ARE supply deficits
    // (a power-only warning must not filter the table down to "no matches").
    setSecPower(true);
    setSecRes(true);
    setKind("all");
    setBal(deficitCount > 0 ? "deficit" : "all");
    setQuery("");
  };
  // Drill-down navigation: select the factory AND fly the map camera to it, so
  // clicking a producer/consumer name takes you straight there.
  const goFactory = (id: Id) => {
    setSelection({ kind: "factory", id });
    const pos = plan.factories[id]?.position;
    if (pos) requestFly(pos);
  };

  const netCell = (net: number) => (
    <span className={`ro-cell ro-net ${net > 0.01 ? "surplus" : net < -0.01 ? "deficit" : "balanced"}`}>
      {Math.abs(net) < 0.01 ? "0" : `${sign(net)}${fmtRate(Math.abs(net))}`}
    </span>
  );

  const ledgerRow = (r: Row) => (
    <>
      <span className="ro-name">
        <ItemIcon item={r.item} displayName={r.label} size={20} />
        <span className="ro-name-text">{r.label}</span>
        {r.raw && <span className="ro-tag-raw">RAW</span>}
      </span>
      <span className="ro-cell ro-make">{r.produced > 1e-6 ? fmtRate(r.produced) : "·"}</span>
      <span className="ro-cell ro-use">{r.consumed > 1e-6 ? fmtRate(r.consumed) : "·"}</span>
      {netCell(r.net)}
    </>
  );

  const drill = (r: Row) => {
    const defs = deficitsByItem.get(r.item) ?? [];
    const netAbs = Math.abs(r.net);
    const solid = (gamedata.items[r.item]?.form ?? "RF_SOLID") === "RF_SOLID";
    return (
      <div className="ro-drill">
        {defs.map((d) => (
          <div className="ro-drill-note" key={`${d.factory}:${d.port}`}>
            ▲ SHORT {fmtRate(d.needed - d.supplied)}/MIN AT {plan.factories[d.factory]?.name ?? "?"} — NEEDS{" "}
            {fmtRate(d.needed)}, SUPPLIED {fmtRate(d.supplied)}
          </div>
        ))}
        {r.makers.length > 0 && (
          <div className="ro-drill-block">
            <div className="ro-drill-h">{r.raw ? "SUPPLIED BY (NODE CLAIMS / EXTERNAL)" : "MADE BY"}</div>
            {r.makers.map((m) => (
              <div className="ro-drill-row" key={m.factory}>
                <button className="ro-fac" onClick={() => goFactory(m.factory)}>
                  {m.name}
                </button>
                <span className="ro-cell ro-make">+{fmtRate(m.rate)}</span>
              </div>
            ))}
          </div>
        )}
        {r.users.length > 0 && (
          <div className="ro-drill-block">
            <div className="ro-drill-h">CONSUMED BY</div>
            {r.users.map((u) => (
              <div className="ro-drill-row" key={u.factory}>
                <button className="ro-fac" onClick={() => goFactory(u.factory)}>
                  {u.name}
                </button>
                <span className="ro-cell ro-use">−{fmtRate(u.rate)}</span>
              </div>
            ))}
          </div>
        )}
        {netAbs > 0.01 && solid && (
          <div className="ro-drill-belt">
            NET {sign(r.net)}
            {fmtRate(netAbs)}/MIN — FITS A MK.{minBeltTier(netAbs)} BELT
          </div>
        )}
      </div>
    );
  };

  const powerBlock = (
    <div className="ro-powerblock">
      <div className="ro-powerblock-top">
        <span className="ro-power-label">⚡ POWER</span>
        <span className={`ro-level ${worst}`}>{LEVEL_WORD[worst]}</span>
      </div>
      {genMw > 0 || drawMw > 0 ? (
        <>
          <div className="ro-net-line">
            <span className={`ro-net-big ${powerNet < -0.01 ? "deficit" : "surplus"}`}>
              {sign(powerNet)}
              {fmtPower(Math.abs(powerNet))}
            </span>
            <span className="ro-headroom">{Math.round(empireHeadroom * 100)}% HEADROOM</span>
          </div>
          <div className="ro-genline">
            GEN {fmtPower(genMw)}&nbsp;&nbsp;·&nbsp;&nbsp;DRAW {fmtPower(drawMw)}
          </div>
          <div className="ro-bar">
            <span className={worst} style={{ width: `${barFill(genMw, drawMw)}%` }} />
          </div>
        </>
      ) : (
        <div className="ro-genline ghost">NO GENERATION · NO DRAW</div>
      )}
    </div>
  );

  return (
    <div className={`resource-overview ro-${ui}`} data-testid="resource-overview">
      {ui === "collapsed" ? (
        <button className="ro-rail" onClick={stepUp} title="Expand resource overview">
          <span className="ro-rail-caret">▸</span>
          <span className="ro-rail-label">RESOURCES</span>
          {deficitCount > 0 && (
            <span className="ro-rail-deficits" title={`${deficitCount} supply deficits`}>
              {deficitCount}
            </span>
          )}
          <span className={`ro-dot ${worst}`} />
          <span className="ro-rail-label sub">POWER</span>
        </button>
      ) : (
        <>
          <div className="ro-headrow">
            <span className="ro-title t-label">OVERVIEW</span>
            <span className="ro-count">{rows.length}</span>
            <span className="ro-flex" />
            {ui === "brief" && (
              <button className="ro-step" onClick={stepUp} title="Full table + grids">
                ⤢
              </button>
            )}
            <button className="ro-step" onClick={stepDown} title={ui === "detailed" ? "Back to brief view" : "Collapse to rail"}>
              ◂
            </button>
          </div>
          {powerBlock}
          {problems.length > 0 && (
            <div className="ro-alert-wrap">
              <button className="ro-alert" onClick={goProblems} title="Show deficits in the full table">
                ▲ {problems.join(" · ")} ▸
              </button>
            </div>
          )}

          {ui === "brief" &&
            (rows.length === 0 ? (
              <div className="ro-empty">No production yet — add a factory and set an output target.</div>
            ) : (
              <>
                <div className="ro-head mono">
                  <span>RESOURCES · BUSIEST</span>
                  <span className="ro-cell">MAKE</span>
                  <span className="ro-cell">USE</span>
                  <span className="ro-cell">NET</span>
                </div>
                <div className="ro-rows">
                  {rows.slice(0, BRIEF_MAX).map((r) => (
                    <div className="ro-row" key={r.item} title={r.label}>
                      {ledgerRow(r)}
                    </div>
                  ))}
                </div>
                <div className="ro-more-wrap">
                  <button className="ro-more" onClick={stepUp}>
                    ALL {rows.length} RESOURCES + GRIDS ▸
                  </button>
                </div>
              </>
            ))}

          {ui === "detailed" && (
            <div className="ro-scroll">
              <button className="ro-sec" onClick={() => setSecPower(!secPower)}>
                <span className="ro-title t-label">POWER</span>
                <span className="ro-count">{derived.circuits.length} GRIDS</span>
                <span className="ro-flex" />
                <span className="ro-sec-caret">{secPower ? "▾" : "▸"}</span>
              </button>
              {secPower && derived.circuits.length === 0 && <div className="ro-sec-empty">NO GRIDS YET</div>}
              {secPower && derived.circuits.length > 0 && (
                <div className="ro-grids">
                  {derived.circuits.map((c: DerivedCircuit) => {
                    const h = circuitHeadroom(c.generationMw, c.demandMw);
                    const lvl = powerLevel(h);
                    return (
                      <div className="ro-grid" key={c.name}>
                        <div className="ro-grid-top">
                          <span className="ro-grid-name">{c.name}</span>
                          <span className={`ro-grid-head ${lvl}`}>
                            {(h * 100).toFixed(h * 100 < 10 ? 1 : 0)}% HEADROOM
                          </span>
                        </div>
                        <div className="ro-bar">
                          <span className={lvl} style={{ width: `${barFill(c.generationMw, c.demandMw)}%` }} />
                        </div>
                        <div className="ro-grid-sub">
                          <span>
                            GEN {fmtPower(c.generationMw)} · DRAW {fmtPower(c.demandMw)}
                          </span>
                          {c.nextShed && <span className={`ro-grid-shed ${lvl}`}>⚠ SHEDS {c.nextShed}</span>}
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
              <button className="ro-sec" onClick={() => setSecRes(!secRes)}>
                <span className="ro-title t-label">RESOURCES</span>
                <span className="ro-count">{rows.length} ITEMS</span>
                <span className="ro-flex" />
                <span className="ro-sec-caret">{secRes ? "▾" : "▸"}</span>
              </button>
              {secRes && (
                <>
              <div className="ro-controls">
                <input
                  className="ro-search"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="FILTER RESOURCES…"
                />
                <div className="ro-chips">
                  {(["all", "raw", "made"] as const).map((k) => (
                    <button key={k} className={`ro-chip ${kind === k ? "active" : ""}`} onClick={() => setKind(k)}>
                      {k.toUpperCase()}
                    </button>
                  ))}
                  <span className="ro-chip-sep" />
                  {(["surplus", "deficit"] as const).map((b) => (
                    <button
                      key={b}
                      className={`ro-chip ${bal === b ? "active" : ""}`}
                      onClick={() => setBal(bal === b ? "all" : b)}
                    >
                      {b.toUpperCase()}
                    </button>
                  ))}
                </div>
              </div>
              {rows.length === 0 ? (
                <div className="ro-empty">No production yet — add a factory and set an output target.</div>
              ) : (
                <>
                  <div className="ro-head mono">
                    <span>ITEM · {shown.length} SHOWN</span>
                    <span className="ro-cell">MAKE</span>
                    <span className="ro-cell">USE</span>
                    <span className="ro-cell">NET</span>
                  </div>
                  <div className="ro-rows">
                    {shown.map((r) => (
                      <div key={r.item}>
                        <button
                          className={`ro-row ro-rowbtn ${expanded === r.item ? "open" : ""}`}
                          title={`${r.label} — click for producing / consuming factories`}
                          onClick={() => setExpanded(expanded === r.item ? null : r.item)}
                        >
                          {ledgerRow(r)}
                        </button>
                        {expanded === r.item && drill(r)}
                      </div>
                    ))}
                    {shown.length === 0 && <div className="ro-empty">No resources match this filter.</div>}
                  </div>
                </>
              )}
                </>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
