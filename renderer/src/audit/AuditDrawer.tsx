// Audit drawer (mock 2d): full-width bottom drawer, half height, pinnable;
// TAB toggles it as a HUD. Tabs carry live count badges; rows re-audit on
// every change (the store is already event-sourced, so rows are always live).

import { useEffect, useMemo, useState } from "react";
import { useStore } from "../state/store";
import {
  fmtPercent,
  fmtPower,
  fmtRate,
  flowBand,
  bottleneckEdges,
  routeBottleneck,
  circuitHeadroom,
  powerLevel,
  type FlowBand,
} from "../lib/format";
import { beltCapacity } from "../state/types";
import type { AltOpportunity } from "../state/types";
import "./audit.css";

type Tab = "saturation" | "deficits" | "power" | "drift" | "optimizer";

/** Cargo route kinds audited for saturation (A3.1): throughput vs demand.
 *  Pipe is excluded — not creatable in the UI, no derived flow/capacity. */
const CARGO_KINDS = ["belt", "rail", "truck", "drone"];

interface SatRow {
  key: string;
  label: string;
  tierText: string;
  saturation: number;
  flow: number;
  capacity: number;
  /** efficiency band (under/good/bottleneck) — shared authority in lib/format */
  band: FlowBand;
  trace: () => void;
  upgrade: (() => void) | null;
}

// PLAN DRIFT (mock 2d + SDD §8): ◇ planned entities awaiting build, plus the
// open SaveReimport proposal's drift rows (game changed since last import).
function DriftTab() {
  const plan = useStore((s) => s.plan);
  const setReviewing = useStore((s) => s.setReviewing);
  const planned = Object.values(plan.factories).filter((f) => f.status === "planned").length;
  // A ◆ built group carrying a ◇ delta is a planned change not yet built in-game.
  const plannedGroups = Object.values(plan.groups).filter(
    (g) => g.status === "planned" || (g.status === "built" && g.plannedDelta != null),
  ).length;
  const reimport = Object.values(plan.proposals).find(
    (p) => p.source === "save_reimport" && (p.status === "draft" || p.status === "reviewing"),
  );
  const hasBuilt = Object.values(plan.factories).some((f) => f.status === "built");
  if (!hasBuilt && planned + plannedGroups === 0) {
    return <div className="drawer-empty">Nothing planned, nothing built — the map is clean.</div>;
  }
  return (
    <>
      {reimport &&
        reimport.items.map((i) => (
          <div className="audit-row" key={i.id} data-testid="drift-row">
            <span className="audit-name">{i.label}</span>
            <span className="mono audit-tier">GAME DRIFT</span>
            <span className="mono audit-load warn">{i.detail}</span>
            <span className="audit-bar" />
            <span className="mono audit-proj">{reimport.title}</span>
            <span className="mono audit-trend">—</span>
            <span className="audit-actions">
              <button className="chip warn" onClick={() => setReviewing(reimport.id)}>
                REVIEW
              </button>
            </span>
          </div>
        ))}
      {planned + plannedGroups > 0 && (
        <div className="audit-row" data-testid="plan-drift-row">
          <span className="audit-name">Plan ahead of the build</span>
          <span className="mono audit-tier">PLAN DRIFT</span>
          <span className="mono audit-load">◇ {planned + plannedGroups}</span>
          <span className="audit-bar" />
          <span className="mono audit-proj">
            {planned} factories · {plannedGroups} machine groups planned, not yet built in-game
          </span>
          <span className="mono audit-trend">—</span>
          <span className="audit-actions" />
        </div>
      )}
      {hasBuilt && !reimport && planned + plannedGroups === 0 && (
        <div className="drawer-empty">Built layer in sync — re-import a save to check for game drift.</div>
      )}
    </>
  );
}

// ALT OPTIMIZER (W2b-D): the empire-wide alternate-recipe ranking. Derived and
// advisory — a read-only fetch off canonical state, empty in the fixture (no
// unlocked alternates). Each row's REVIEW CTA drafts the change into the
// existing review surface: an all-◇ opportunity → a T2 SetGroupRecipe proposal;
// any ◆ built factory → a W2a Refactor (the ◆ layer is never mutated).
function OptimizerTab() {
  const optimizeEmpire = useStore((s) => s.optimizeEmpire);
  const optimizeAdopt = useStore((s) => s.optimizeAdopt);
  const gamedata = useStore((s) => s.gamedata);
  const planHash = useStore((s) => s.planHash);
  const [rows, setRows] = useState<AltOpportunity[] | null>(null);
  const itemName = (cls: string) => gamedata.items[cls]?.displayName ?? cls;

  // Re-fetch whenever the plan content changes (planHash) — the ranking is a
  // pure function of state + gamedata + unlocked, so it re-audits like the rest.
  useEffect(() => {
    let live = true;
    void optimizeEmpire().then((r) => {
      if (live) setRows(r);
    });
    return () => {
      live = false;
    };
  }, [optimizeEmpire, planHash]);

  if (rows === null) return <div className="drawer-empty">Ranking alternates…</div>;
  if (rows.length === 0) {
    return (
      <div className="drawer-empty">
        No unlocked alternates to weigh — import a save with hard-drive unlocks and the optimizer ranks the
        machine/power savings of adopting them empire-wide.
      </div>
    );
  }
  return (
    <>
      {rows.map((o) => (
        <div className="audit-row" key={o.recipe} data-testid="optimizer-row">
          <span className="audit-name">
            {o.recipeName} · {o.productName}
          </span>
          <span className="mono audit-tier">ALT · {o.retoolEstHours > 0 ? `~${o.retoolEstHours.toFixed(1)}H RETOOL` : "◇ FREE"}</span>
          <span className="mono audit-load ok">
            −{o.machinesSaved} machines / −{fmtPower(o.powerSavedMw)}
          </span>
          <span className="audit-bar" />
          <span className="mono audit-proj projected">
            {o.affectedPlanned.length > 0 && <>◇ {o.affectedPlanned.length} planned </>}
            {o.affectedBuilt.length > 0 && <>◆ {o.affectedBuilt.length} built </>}
            {o.inputDeltas.map(([item, delta]) => (
              <span className={`chip ${delta > 0 ? "warn" : ""}`} key={item} style={{ marginLeft: 4 }}>
                {delta > 0 ? "+" : "−"}
                {fmtRate(Math.abs(delta))} {itemName(item)}
              </span>
            ))}
            {o.nodeReuse && (
              <span className="chip warn" style={{ marginLeft: 4 }}>
                ⚠ NODE REUSE — build-window downtime
              </span>
            )}
          </span>
          <span className="mono audit-trend">—</span>
          <span className="audit-actions">
            <button className="chip warn" onClick={() => void optimizeAdopt(o.recipe)}>
              REVIEW
            </button>
          </span>
        </div>
      ))}
    </>
  );
}

export default function AuditDrawer({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const setSelection = useStore((s) => s.setSelection);
  const setView = useStore((s) => s.setView);
  const dispatch = useStore((s) => s.dispatch);
  const setWizard = useStore((s) => s.setWizard);
  const [tab, setTab] = useState<Tab>("saturation");
  const [pinned, setPinned] = useState(false);

  const itemName = (cls: string) => gamedata.items[cls]?.displayName ?? cls;

  const satRows: SatRow[] = useMemo(() => {
    const rows: SatRow[] = [];
    for (const r of Object.values(plan.routes)) {
      if (!CARGO_KINDS.includes(r.kind.kind)) continue;
      const d = derived.routes[r.id];
      if (!d) continue;
      const src = plan.ports[r.endpoints[0]];
      const dst = plan.ports[r.endpoints[1]];
      const tier = r.kind.kind === "belt" ? r.kind.tier : null;
      rows.push({
        key: `route-${r.id}`,
        label: `${src ? plan.factories[src.factory]?.name ?? "?" : "?"} ⟶ ${
          dst ? plan.factories[dst.factory]?.name ?? "?" : "?"
        } · ${itemName(r.manifest[0]?.[0] ?? "")}`,
        tierText: `ROUTE · ${tier != null ? `MK.${tier}` : r.kind.kind.toUpperCase()}`,
        saturation: d.saturation,
        flow: d.flow,
        capacity: d.capacity,
        band: flowBand(d.saturation, d.flow, routeBottleneck(r.id, d.saturation, derived.deficits)),
        trace: () => {
          setView({ mode: "map" });
          setSelection({ kind: "route", id: r.id });
        },
        // UPGRADE TIER is belt-specific; consist/fleet stepping for transports
        // lives in the TransportDrawer.
        upgrade:
          tier != null && tier < 6
            ? () => void dispatch([{ type: "set_route_tier", id: r.id, tier: tier + 1 }])
            : null,
      });
    }
    // Solver-named capacity bindings per factory (the honest bottleneck red).
    const bottlenecksByFactory = new Map<string, Set<string>>();
    for (const e of Object.values(plan.edges)) {
      const df = derived.factories[e.factory];
      const d = df?.edges[e.id];
      if (!d) continue;
      let bset = bottlenecksByFactory.get(e.factory);
      if (!bset) {
        bset = bottleneckEdges(df);
        bottlenecksByFactory.set(e.factory, bset);
      }
      const toGroup = e.to.kind === "group" ? plan.groups[e.to.id] : undefined;
      const toName = toGroup
        ? gamedata.machines[toGroup.machine]?.displayName?.toUpperCase()
        : e.to.kind === "port"
          ? "PORT"
          : e.to.kind === "junction"
            ? "JUNCTION"
            : undefined;
      rows.push({
        key: `edge-${e.id}`,
        label: `${plan.factories[e.factory]?.name ?? "?"} · ${itemName(e.item)}${toName ? ` → ${toName}` : ""}`,
        tierText: `BELT · MK.${e.tier}`,
        saturation: d.saturation,
        flow: d.flow,
        capacity: beltCapacity(e.tier),
        band: flowBand(d.saturation, d.flow, bset.has(e.id)),
        trace: () => {
          setView({ mode: "factory", factoryId: e.factory });
          setSelection({ kind: "edge", id: e.id });
        },
        upgrade:
          e.tier < 6 ? () => void dispatch([{ type: "set_edge_tier", id: e.id, tier: e.tier + 1 }]) : null,
      });
    }
    // Efficiency ranking: bottlenecks first (the real problems), then the
    // under-used (waste / starved upstream), then good; utilization desc within.
    const rank: Record<FlowBand, number> = { bottleneck: 2, under: 1, good: 0 };
    return rows.sort((a, b) => rank[b.band] - rank[a.band] || b.saturation - a.saturation);
  }, [plan, derived, dispatch, setSelection, setView, gamedata.items]);

  // Badge = the ALARM channel: bottlenecks only, matching the status bar's
  // bottleneck-only ⚠. Under-used rows are waste — listed (and ranked) in the
  // tab, never alarmed; counting them branded healthy bases with triple-digit
  // badges. The ≤50% boundary itself is user-pinned (see DECISIONS
  // efficiency-grammar-completion + its OPEN Mk1-amber question).
  const alarmCount = satRows.filter((r) => r.band === "bottleneck").length;
  const deficitCount = derived.deficits.length + Object.values(derived.nodes).filter((n) => n.conflict).length;
  const powerRows = useMemo(
    () =>
      Object.entries(derived.factories)
        .map(([fid, df]) => ({ fid, name: plan.factories[fid]?.name ?? "?", mw: df.totalPowerMw }))
        .filter((r) => r.mw > 0)
        .sort((a, b) => b.mw - a.mw),
    [derived.factories, plan.factories],
  );
  // Circuit margins (SDD §12): headroom ≥20% OK, 5–20% WARN, <5% CRIT.
  const circuitRows = useMemo(
    () =>
      derived.circuits
        .map((c) => {
          const headroom = circuitHeadroom(c.generationMw, c.demandMw);
          const level = powerLevel(headroom);
          return { ...c, headroom, level };
        })
        .sort((a, b) => a.headroom - b.headroom),
    [derived.circuits],
  );
  const powerBadge = circuitRows.filter((c) => c.level !== "ok").length;

  if (!open) {
    return (
      <button className="audit-handle mono" onClick={onToggle} data-testid="audit-handle">
        ▲ AUDIT (TAB)
        {alarmCount + deficitCount > 0 && <span className="audit-badge">{alarmCount + deficitCount}</span>}
      </button>
    );
  }

  return (
    <div className={`audit-drawer ${pinned ? "pinned" : ""}`} data-testid="audit-drawer">
      <header className="audit-head">
        {(
          [
            ["saturation", `SATURATION`, alarmCount],
            ["deficits", `DEFICITS`, deficitCount],
            ["power", `POWER`, powerBadge],
            ["drift", `PLAN DRIFT`, 0],
            ["optimizer", `ALT OPTIMIZER`, 0],
          ] as const
        ).map(([id, label, count]) => (
          <button key={id} className={`audit-tab t-label ${tab === id ? "active" : ""}`} onClick={() => setTab(id)}>
            {label}
            {count > 0 && <span className="audit-badge">{count}</span>}
          </button>
        ))}
        <span className="mono audit-live">
          LIVE — re-audits on every change · {(derived.recomputeUs / 1000).toFixed(1)}ms
        </span>
        <button className={`btn btn-ghost audit-pin ${pinned ? "active" : ""}`} onClick={() => setPinned(!pinned)}>
          PIN
        </button>
        <button className="drawer-close" onClick={onToggle} aria-label="Close">
          ×
        </button>
      </header>

      <div className="audit-body">
        {tab === "saturation" && (
          <>
            {satRows.length === 0 && <div className="drawer-empty">No routes yet.</div>}
            {satRows.map((r) => (
              // Efficiency grammar: red = BOTTLENECK (solver-named cap), amber
              // = under-used (≤50% while flowing), green = good — a FULL belt
              // meeting demand is optimal, so 100% alone stays green.
              <div className={`audit-row ${r.band === "bottleneck" ? "crit" : ""}`} key={r.key}>
                <span className="audit-name">{r.label}</span>
                <span className="mono audit-tier">{r.tierText}</span>
                <span
                  className={`mono audit-load ${r.band}`}
                  title={
                    r.band === "bottleneck"
                      ? "Bottleneck — this link runs full while downstream demand goes unmet"
                      : r.band === "under"
                        ? "Under-used — flowing at ≤50% of rated capacity (over-built or starved upstream)"
                        : r.flow === 0
                          ? "Idle — no derived flow through this link"
                          : r.saturation >= 0.999
                            ? "Good — a full belt that meets demand is optimal"
                            : "Good — >50% utilized"
                  }
                >
                  {fmtPercent(r.saturation)}
                  {r.band === "under" ? " UNDER" : r.band === "bottleneck" ? " ⚠" : ""}
                </span>
                <span className="audit-bar">
                  <span className={r.band} style={{ width: `${Math.min(100, r.saturation * 100)}%` }} />
                </span>
                <span className="mono audit-proj projected">
                  {fmtRate(r.flow)}/{fmtRate(r.capacity)}
                </span>
                <span className="mono audit-trend">—</span>
                <span className="audit-actions">
                  <button className="chip" onClick={r.trace}>
                    TRACE
                  </button>
                  {r.upgrade && r.band === "bottleneck" && (
                    <button className="chip warn" onClick={r.upgrade}>
                      UPGRADE TIER
                    </button>
                  )}
                </span>
              </div>
            ))}
          </>
        )}

        {tab === "deficits" && (
          <>
            {deficitCount === 0 && <div className="drawer-empty">No deficits — every target is fed.</div>}
            {derived.deficits.map((d) => (
              <div className="audit-row crit" key={`${d.factory}-${d.port}`}>
                <span className="audit-name">
                  {plan.factories[d.factory]?.name ?? "?"} starved of {itemName(d.item)}
                </span>
                <span className="mono audit-tier">DEFICIT</span>
                <span className="mono audit-load crit">
                  −{fmtRate(d.needed - d.supplied)}
                  <span className="unit">/min</span>
                </span>
                <span className="audit-bar">
                  <span className="crit" style={{ width: `${Math.min(100, (d.supplied / d.needed) * 100)}%` }} />
                </span>
                <span className="mono audit-proj">
                  {fmtRate(d.supplied)} of {fmtRate(d.needed)} supplied
                </span>
                <span className="mono audit-trend">—</span>
                <span className="audit-actions">
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
                      }
                    }}
                  >
                    TRACE
                  </button>
                </span>
              </div>
            ))}
            {Object.entries(derived.nodes)
              .filter(([, n]) => n.conflict)
              .map(([nodeId, n]) => (
                <div className="audit-row crit" key={nodeId}>
                  <span className="audit-name">Node {nodeId} double-booked</span>
                  <span className="mono audit-tier">CONFLICT</span>
                  <span className="mono audit-load crit">⚠×{n.claims}</span>
                  <span className="audit-bar" />
                  <span className="mono audit-proj">combined claims exceed extraction</span>
                  <span className="mono audit-trend">—</span>
                  <span className="audit-actions">
                    <button
                      className="chip"
                      onClick={() => {
                        setView({ mode: "map" });
                        setSelection({ kind: "node", id: nodeId });
                      }}
                    >
                      TRACE
                    </button>
                  </span>
                </div>
              ))}
          </>
        )}

        {tab === "power" && (
          <>
            {(derived.totalPowerMw > 0 || derived.totalGenerationMw > 0) &&
              (() => {
                // Empire balance up top — the question a player actually asks:
                // am I generating enough, and how much headroom before a
                // brownout? Circuits below break it down per grid; this stays
                // meaningful even for an imported empire with no derived grids.
                const gen = derived.totalGenerationMw;
                const draw = derived.totalPowerMw;
                const net = gen - draw;
                const headroom = circuitHeadroom(gen, draw);
                const level = gen > 0 ? powerLevel(headroom) : draw > 0 ? "warn" : "ok";
                return (
                  <div className={`audit-row power-summary ${level === "crit" ? "crit" : ""}`} data-testid="power-summary">
                    <span className="audit-name">Empire power</span>
                    <span className="mono audit-tier">BALANCE</span>
                    <span className={`mono audit-load ${level}`}>
                      {gen > 0 ? fmtPercent(headroom) : "NO GEN"}
                    </span>
                    <span className="audit-bar">
                      <span
                        className={level === "ok" ? "" : level}
                        style={{ width: `${Math.min(100, gen > 0 ? (draw / gen) * 100 : 100)}%` }}
                      />
                    </span>
                    <span className="mono audit-proj projected">
                      {gen > 0 ? (
                        <>
                          {fmtPower(draw)} draw of {fmtPower(gen)} generated · {net >= 0 ? "+" : "−"}
                          {fmtPower(Math.abs(net))} headroom
                        </>
                      ) : (
                        <>{fmtPower(draw)} draw · generation not captured (no grid in this import)</>
                      )}
                    </span>
                    <span className="mono audit-trend">—</span>
                    <span className="audit-actions" />
                  </div>
                );
              })()}
            {circuitRows.length === 0 && powerRows.length === 0 && derived.totalGenerationMw === 0 && (
              <div className="drawer-empty">No powered machines yet — grids appear once power lines join factories.</div>
            )}
            {circuitRows.map((c) => (
              <div className={`audit-row ${c.level === "crit" ? "crit" : ""}`} key={c.name}>
                <span className="audit-name">
                  {c.name} · {c.members.map((m) => plan.factories[m]?.name ?? "?").join(" + ")}
                </span>
                <span className="mono audit-tier">CIRCUIT</span>
                <span className={`mono audit-load ${c.level}`}>
                  {c.generationMw > 0 ? fmtPercent(c.headroom) : "NO GEN"}
                </span>
                <span className="audit-bar">
                  <span
                    className={c.level === "ok" ? "" : c.level}
                    style={{
                      width: `${Math.min(100, c.generationMw > 0 ? (c.demandMw / c.generationMw) * 100 : 100)}%`,
                    }}
                  />
                </span>
                <span className="mono audit-proj projected">
                  {fmtPower(c.demandMw)} of {fmtPower(c.generationMw)} generated
                </span>
                <span className="mono audit-trend">—</span>
                <span className="audit-actions">
                  <button
                    className="chip"
                    onClick={() => {
                      const first = c.members[0];
                      if (first) {
                        setView({ mode: "map" });
                        setSelection({ kind: "factory", id: first });
                      }
                    }}
                  >
                    TRACE
                  </button>
                </span>
              </div>
            ))}
            {circuitRows.map(
              (c) =>
                c.switches.length > 0 && (
                  <div key={`${c.name}-shed`}>
                    <div className="audit-row" data-testid="brownout-row">
                      <span className="audit-name" style={{ color: "var(--ink-500)" }}>
                        {c.name} · BROWNOUT SIM — next shed: {c.nextShed}
                      </span>
                      <span className="mono audit-tier">SIM</span>
                      <span className="mono audit-load" />
                      <span className="audit-bar" />
                      <span className="mono audit-proj" />
                      <span className="mono audit-trend">—</span>
                      <span className="audit-actions">
                        <button
                          className="chip"
                          onClick={() => {
                            const first = c.switches[0];
                            if (first) {
                              setView({ mode: "map" });
                              setSelection({ kind: "switch", id: first.id });
                            }
                          }}
                        >
                          TRACE
                        </button>
                      </span>
                    </div>
                    {c.switches.map((sw) => (
                      <div className="audit-row" key={sw.id} data-testid="switch-row">
                        <span className="audit-name">
                          {c.name} switch · sheds {fmtPower(sw.downstreamMw)}
                        </span>
                        <span className="mono audit-tier">PRIORITY P{sw.priority}</span>
                        <span className="mono audit-load">{fmtPower(sw.shedsAtMw)}</span>
                        <span className="audit-bar">
                          <span
                            className={c.demandMw >= sw.shedsAtMw ? "crit" : ""}
                            style={{ width: `${Math.min(100, (c.demandMw / Math.max(1, sw.shedsAtMw)) * 100)}%` }}
                          />
                        </span>
                        <span className="mono audit-proj projected">SHEDS AT {fmtPower(sw.shedsAtMw)}</span>
                        <span className="mono audit-trend">—</span>
                        <span className="audit-actions">
                          <button
                            className="chip"
                            onClick={() => {
                              setView({ mode: "map" });
                              setSelection({ kind: "switch", id: sw.id });
                            }}
                          >
                            TRACE
                          </button>
                        </span>
                      </div>
                    ))}
                  </div>
                ),
            )}
            {powerRows.map((r) => (
              <div className="audit-row" key={r.fid}>
                <span className="audit-name">{r.name}</span>
                <span className="mono audit-tier">DRAW</span>
                <span className="mono audit-load">{fmtPower(r.mw)}</span>
                <span className="audit-bar">
                  <span style={{ width: `${Math.min(100, (r.mw / Math.max(1, derived.totalPowerMw)) * 100)}%` }} />
                </span>
                <span className="mono audit-proj projected">of {fmtPower(derived.totalPowerMw)} empire draw</span>
                <span className="mono audit-trend">—</span>
                <span className="audit-actions">
                  <button
                    className="chip"
                    onClick={() => {
                      setView({ mode: "factory", factoryId: r.fid });
                    }}
                  >
                    TRACE
                  </button>
                </span>
              </div>
            ))}
          </>
        )}

        {tab === "drift" && <DriftTab />}

        {tab === "optimizer" && <OptimizerTab />}
      </div>
      <footer className="audit-foot mono">sorted by severity · rows re-audit live</footer>
    </div>
  );
}
