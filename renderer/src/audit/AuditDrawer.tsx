// Audit drawer (mock 2d): full-width bottom drawer, half height, pinnable;
// TAB toggles it as a HUD. Tabs carry live count badges; rows re-audit on
// every change (the store is already event-sourced, so rows are always live).

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import { fmtPercent, fmtPower, fmtRate, flowLevel } from "../lib/format";
import { beltCapacity } from "../state/types";
import "./audit.css";

type Tab = "saturation" | "deficits" | "power" | "drift";

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
    for (const e of Object.values(plan.edges)) {
      const df = derived.factories[e.factory];
      const d = df?.edges[e.id];
      if (!d) continue;
      rows.push({
        key: `edge-${e.id}`,
        label: `${plan.factories[e.factory]?.name ?? "?"} · ${itemName(e.item)}`,
        tierText: `BELT · MK.${e.tier}`,
        saturation: d.saturation,
        flow: d.flow,
        capacity: beltCapacity(e.tier),
        trace: () => {
          setView({ mode: "factory", factoryId: e.factory });
          setSelection({ kind: "edge", id: e.id });
        },
        upgrade:
          e.tier < 6 ? () => void dispatch([{ type: "set_edge_tier", id: e.id, tier: e.tier + 1 }]) : null,
      });
    }
    return rows.sort((a, b) => b.saturation - a.saturation);
  }, [plan, derived, dispatch, setSelection, setView, gamedata.items]);

  const hotCount = satRows.filter((r) => r.saturation >= 0.7).length;
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
          const headroom = c.generationMw > 0 ? (c.generationMw - c.demandMw) / c.generationMw : c.demandMw > 0 ? -1 : 1;
          const level: "ok" | "warn" | "crit" = headroom < 0.05 ? "crit" : headroom < 0.2 ? "warn" : "ok";
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
        {hotCount + deficitCount > 0 && <span className="audit-badge">{hotCount + deficitCount}</span>}
      </button>
    );
  }

  return (
    <div className={`audit-drawer ${pinned ? "pinned" : ""}`} data-testid="audit-drawer">
      <header className="audit-head">
        {(
          [
            ["saturation", `SATURATION`, hotCount],
            ["deficits", `DEFICITS`, deficitCount],
            ["power", `POWER`, powerBadge],
            ["drift", `PLAN DRIFT`, 0],
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
            {satRows.map((r) => {
              const level = flowLevel(r.saturation);
              return (
                <div className={`audit-row ${level === "crit" ? "crit" : ""}`} key={r.key}>
                  <span className="audit-name">{r.label}</span>
                  <span className="mono audit-tier">{r.tierText}</span>
                  <span className={`mono audit-load ${level}`}>{fmtPercent(r.saturation)}</span>
                  <span className="audit-bar">
                    <span className={level} style={{ width: `${Math.min(100, r.saturation * 100)}%` }} />
                  </span>
                  <span className="mono audit-proj projected">
                    {fmtRate(r.flow)}/{fmtRate(r.capacity)}
                  </span>
                  <span className="mono audit-trend">—</span>
                  <span className="audit-actions">
                    <button className="chip" onClick={r.trace}>
                      TRACE
                    </button>
                    {r.upgrade && r.saturation >= 0.7 && (
                      <button className="chip warn" onClick={r.upgrade}>
                        UPGRADE TIER
                      </button>
                    )}
                  </span>
                </div>
              );
            })}
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
            {circuitRows.length === 0 && powerRows.length === 0 && (
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
      </div>
      <footer className="audit-foot mono">sorted by severity · rows re-audit live</footer>
    </div>
  );
}
