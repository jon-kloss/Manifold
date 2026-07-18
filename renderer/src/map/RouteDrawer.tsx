// Route inspector (belt routes, Phase 2). A3 grammar: every route is an
// entity with an inspector; planned routes are ◇ with italic projections.
// The full rail/truck/drone math block arrives in Phase 4.

import { useStore } from "../state/store";
import { fmtClockS, fmtKm, fmtPercent, fmtPower, fmtRate, flowBand, routeBottleneck, itemLabel } from "../lib/format";
import { DEFAULT_DRONE_SPEC, DEFAULT_RAIL_SPEC, DEFAULT_TRUCK_SPEC } from "../state/types";
import type { RailSpec, Route, RouteKind } from "../state/types";
import TrainAnswerBlock from "./TrainAnswerBlock";
import { trainAnswerFromMath } from "./trainAnswer";
import ItemIcon from "../lib/ItemIcon";

export default function RouteDrawer({ route }: { route: Route }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);

  if (route.kind.kind === "power") return <PowerLineDrawer route={route} />;
  if (route.kind.kind === "rail" || route.kind.kind === "truck" || route.kind.kind === "drone") {
    return <TransportDrawer route={route} />;
  }

  const dr = derived.routes[route.id];
  const srcPort = plan.ports[route.endpoints[0]];
  const dstPort = plan.ports[route.endpoints[1]];
  const srcFactory = srcPort ? plan.factories[srcPort.factory] : null;
  const dstFactory = dstPort ? plan.factories[dstPort.factory] : null;
  const tier = route.kind.kind === "belt" ? route.kind.tier : 0;
  const sat = dr?.saturation ?? 0;
  // Efficiency band (shared authority in lib/format): amber = under-used,
  // red = bottleneck (deficit through a full route); a full belt meeting
  // demand stays quiet — optimal.
  const band = flowBand(sat, dr?.flow ?? 0, routeBottleneck(route.id, sat, derived.deficits));
  const level = band === "good" ? "" : band;

  // Header tile: the routed item (single-item manifests in v1). An empty
  // manifest keeps the placeholder — there is nothing honest to show.
  const routedItem = route.manifest[0]?.[0] ?? null;

  return (
    <aside className="drawer summary-drawer" data-testid="route-drawer">
      <header className="drawer-header">
        {routedItem ? (
          <ItemIcon item={routedItem} displayName={gamedata.items[routedItem]?.displayName} size={40} />
        ) : (
          <div className="icon-ph s40" />
        )}
        <div className="drawer-title-block">
          <div className="t-title">BELT ROUTE</div>
          <div className="mono drawer-sub">
            {dr ? fmtKm(dr.lengthM) : "—"} · MK.{tier} · {fmtRate(dr?.capacity ?? 0)}/min CAP
          </div>
        </div>
        {route.status === "built" ? (
          <span className="chip built">◆ BUILT</span>
        ) : (
          <span className="chip planned">◇ PLANNED</span>
        )}
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      <section className="drawer-section">
        <h3 className="t-label">ENDPOINTS</h3>
        <div className="drawer-row">
          <button
            className="chip"
            onClick={() => srcFactory && setSelection({ kind: "factory", id: srcFactory.id })}
          >
            ◇ {srcFactory?.name.toUpperCase() ?? "?"}
          </button>
          <span className="mono" style={{ color: "var(--ink-500)" }}>
            ⟶
          </span>
          <button
            className="chip"
            onClick={() => dstFactory && setSelection({ kind: "factory", id: dstFactory.id })}
          >
            ◇ {dstFactory?.name.toUpperCase() ?? "?"}
          </button>
        </div>
      </section>

      <section className="drawer-section">
        <h3 className="t-label">MANIFEST</h3>
        {route.manifest.map(([item, rate]) => (
          <div className="drawer-row" key={item}>
            <ItemIcon item={item} displayName={gamedata.items[item]?.displayName} size={20} />
            <span className="drawer-row-name">{itemLabel(gamedata.items, item)}</span>
            <span className="t-data-12 projected">
              {fmtRate(rate)}
              <span className="unit">/min</span>
            </span>
          </div>
        ))}
      </section>

      <section className="drawer-section">
        <h3 className="t-label">LOAD</h3>
        <KindSwitcher route={route} />
        <div className="drawer-row">
          <span className="drawer-row-name">Belt tier</span>
          {route.status === "built" ? (
            <span className="mono t-data-12" data-testid="route-tier-built" title="Imported as built — rebuild in-game to change its tier.">
              MK.{tier} · BUILT
            </span>
          ) : (
            <select
              className="mono"
              style={{ height: 24 }}
              value={tier}
              onChange={(e) => void dispatch([{ type: "set_route_tier", id: route.id, tier: Number(e.target.value) }])}
              data-testid="route-tier-select"
            >
              {[1, 2, 3, 4, 5, 6].map((t) => (
                <option key={t} value={t}>
                  MK.{t}
                </option>
              ))}
            </select>
          )}
        </div>
        {route.status === "built" && (
          <div className="insp-note">Imported as built — this route's tier is fixed to your save until you rebuild it in-game and re-import.</div>
        )}
        {dr && dr.climbUpM + dr.climbDownM > 0.5 && (
          <div className="drawer-row">
            <span className="drawer-row-name">Climb</span>
            <span className="t-data-12 projected">
              ↑{Math.round(dr.climbUpM)}
              <span className="unit">m</span> ↓{Math.round(dr.climbDownM)}
              <span className="unit">m</span>
            </span>
          </div>
        )}
        <div className="drawer-row">
          <span className="drawer-row-name">Throughput</span>
          <span className="minibar">
            <span className={level} style={{ width: `${Math.min(100, sat * 100)}%` }} />
          </span>
          <span className={`t-data-12 projected ${level ? level : ""}`}>
            {fmtRate(dr?.flow ?? 0)}/{fmtRate(dr?.capacity ?? 0)} · {fmtPercent(sat)}
          </span>
        </div>
        {dr && dr.supplied > dr.flow + 1e-6 && (
          <div className="insp-note">
            Upstream ships {fmtRate(dr.supplied)}/min; the consumer draws {fmtRate(dr.flow)} — slack stays on the
            belt.
          </div>
        )}
      </section>

      <footer className="drawer-footer">
        <button
          className="btn btn-ghost"
          onClick={() => {
            setSelection(null);
            void dispatch([{ type: "delete_route", id: route.id }]);
          }}
        >
          DELETE ROUTE
        </button>
      </footer>
    </aside>
  );
}

// Power line: no manifest, no tier — it joins two factories into one circuit.
// The inspector shows the resulting grid's margin (A2.1: power is a bus).
function PowerLineDrawer({ route }: { route: Route }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);

  const [aId, bId] = route.endpoints;
  const a = plan.factories[aId];
  const b = plan.factories[bId];
  const circuit = derived.circuits.find((c) => c.members.includes(aId));
  const gen = circuit?.generationMw ?? 0;
  const demand = circuit?.demandMw ?? 0;
  const headroom = gen > 0 ? (gen - demand) / gen : demand > 0 ? -1 : 1;
  const level = headroom < 0.05 ? "crit" : headroom < 0.2 ? "warn" : "";

  return (
    <aside className="drawer summary-drawer" data-testid="route-drawer">
      <header className="drawer-header">
        <div className="icon-ph s40" />
        <div className="drawer-title-block">
          <div className="t-title">POWER LINE</div>
          <div className="mono drawer-sub">{circuit?.name ?? "UNGRIDDED"}</div>
        </div>
        <span className="chip planned">◇ PLANNED</span>
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      <section className="drawer-section">
        <h3 className="t-label">ENDPOINTS</h3>
        <div className="drawer-row">
          <button className="chip" onClick={() => a && setSelection({ kind: "factory", id: a.id })}>
            ◇ {a?.name.toUpperCase() ?? "?"}
          </button>
          <span className="mono" style={{ color: "var(--ink-500)" }}>
            ⚡
          </span>
          <button className="chip" onClick={() => b && setSelection({ kind: "factory", id: b.id })}>
            ◇ {b?.name.toUpperCase() ?? "?"}
          </button>
        </div>
      </section>

      <section className="drawer-section">
        <h3 className="t-label">CIRCUIT</h3>
        <div className="drawer-row">
          <span className="drawer-row-name">Generation</span>
          <span className="t-data-12 projected">{fmtPower(gen)}</span>
        </div>
        <div className="drawer-row">
          <span className="drawer-row-name">Demand</span>
          <span className="t-data-12 projected">{fmtPower(demand)}</span>
        </div>
        <div className="drawer-row">
          <span className="drawer-row-name">Margin</span>
          <span className="minibar">
            <span className={level} style={{ width: `${Math.min(100, gen > 0 ? (demand / gen) * 100 : 100)}%` }} />
          </span>
          <span className={`t-data-12 projected ${level}`}>
            {gen > 0 ? fmtPercent(headroom) : "—"} headroom
          </span>
        </div>
        {circuit && circuit.members.length > 2 && (
          <div className="insp-note">{circuit.members.length} factories share this grid.</div>
        )}
      </section>

      <footer className="drawer-footer">
        <button
          className="btn btn-ghost"
          onClick={() =>
            void dispatch([{ type: "add_priority_switch", route: route.id, priority: 4 }], { select: true })
          }
          data-testid="btn-add-switch"
        >
          + PRIORITY SWITCH
        </button>
        <button
          className="btn btn-ghost"
          onClick={() => {
            setSelection(null);
            void dispatch([{ type: "delete_route", id: route.id }]);
          }}
        >
          DELETE LINE
        </button>
      </footer>
    </aside>
  );
}

// Cargo kind switcher — belt↔rail↔truck↔drone keep the same port binding.
function KindSwitcher({ route }: { route: Route }) {
  const dispatch = useStore((s) => s.dispatch);
  const swap = (kind: string) => {
    const next: RouteKind =
      kind === "belt"
        ? { kind: "belt", tier: 3 }
        : kind === "rail"
          ? { kind: "rail", spec: { ...DEFAULT_RAIL_SPEC } }
          : kind === "truck"
            ? { kind: "truck", spec: { ...DEFAULT_TRUCK_SPEC } }
            : { kind: "drone", spec: { ...DEFAULT_DRONE_SPEC } };
    void dispatch([{ type: "set_route_spec", id: route.id, kind: next }]);
  };
  return (
    <div className="drawer-row">
      <span className="drawer-row-name">Transport</span>
      <select
        className="mono"
        style={{ height: 24 }}
        value={route.kind.kind}
        onChange={(e) => swap(e.target.value)}
        data-testid="route-kind-select"
      >
        <option value="belt">BELT</option>
        <option value="rail">RAIL</option>
        <option value="truck">TRUCK</option>
        <option value="drone">DRONE</option>
      </select>
    </div>
  );
}

// Rail/truck/drone inspector (A3.1/A3.2): manifest, consists steppers, and
// THE MATH BLOCK — render it, don't hide it. Throughput vs demand carries the
// flow color and drives the route's map encoding.
function TransportDrawer({ route }: { route: Route }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);

  const dr = derived.routes[route.id];
  const t = dr?.transport ?? null;
  const srcPort = plan.ports[route.endpoints[0]];
  const dstPort = plan.ports[route.endpoints[1]];
  const srcFactory = srcPort ? plan.factories[srcPort.factory] : null;
  const dstFactory = dstPort ? plan.factories[dstPort.factory] : null;
  const kind = route.kind.kind as "rail" | "truck" | "drone";
  const title = kind === "rail" ? "RAIL ROUTE" : kind === "truck" ? "TRUCK ROUTE" : "DRONE ROUTE";
  const demand = dr?.flow ?? 0;
  const throughput = t?.throughputPerMin ?? 0;
  const short = throughput > 0 && demand > throughput + 1e-6;
  // Shared efficiency grammar (lib/format) — the same banding as the map
  // polyline behind this drawer: a 96%-loaded rail meeting demand reads ✓
  // here exactly as it reads green on the map. "crit" is reserved for honest
  // shortfall evidence: throughput short of demand, or a downstream deficit
  // routed through a full link.
  const bn = short || routeBottleneck(route.id, dr?.saturation ?? 0, derived.deficits);
  const band = flowBand(dr?.saturation ?? 0, demand, bn);
  // idle (zero demand) stays neutral — never the healthy ✓, matching the dim
  // idle polyline on the map behind this drawer.
  const level = band === "bottleneck" ? "crit" : band === "under" ? "warn" : band === "idle" ? "idle" : "ok";

  const respec = (patch: Record<string, unknown>) => {
    if (route.kind.kind === "rail") {
      void dispatch([
        { type: "set_route_spec", id: route.id, kind: { kind: "rail", spec: { ...route.kind.spec, ...patch } } },
      ]);
    } else if (route.kind.kind === "truck") {
      void dispatch([
        { type: "set_route_spec", id: route.id, kind: { kind: "truck", spec: { ...route.kind.spec, ...patch } } },
      ]);
    } else if (route.kind.kind === "drone") {
      void dispatch([
        { type: "set_route_spec", id: route.id, kind: { kind: "drone", spec: { ...route.kind.spec, ...patch } } },
      ]);
    }
  };

  const rail: RailSpec | null = route.kind.kind === "rail" ? route.kind.spec : null;
  const truck = route.kind.kind === "truck" ? route.kind.spec : null;
  const drone = route.kind.kind === "drone" ? route.kind.spec : null;
  // Header tile: the routed item (rail manifests are single-item in v1).
  const routedItem = route.manifest[0]?.[0] ?? null;

  return (
    <aside className="drawer summary-drawer" data-testid="route-drawer">
      <header className="drawer-header">
        {routedItem ? (
          <ItemIcon item={routedItem} displayName={gamedata.items[routedItem]?.displayName} size={40} />
        ) : (
          <div className="icon-ph s40" />
        )}
        <div className="drawer-title-block">
          <div className="t-title">{title}</div>
          <div className="mono drawer-sub">
            {t ? fmtKm(t.effectiveLengthM) : "—"}
            {kind !== "drone" && " · ×1.12 TERRAIN"}
          </div>
        </div>
        <span className="chip planned">◇ PLANNED</span>
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      <section className="drawer-section">
        <h3 className="t-label">ENDPOINTS</h3>
        <div className="drawer-row">
          <button className="chip" onClick={() => srcFactory && setSelection({ kind: "factory", id: srcFactory.id })}>
            ◇ {srcFactory?.name.toUpperCase() ?? "?"}
          </button>
          <span className="mono" style={{ color: "var(--ink-500)" }}>
            ⟶
          </span>
          <button className="chip" onClick={() => dstFactory && setSelection({ kind: "factory", id: dstFactory.id })}>
            ◇ {dstFactory?.name.toUpperCase() ?? "?"}
          </button>
        </div>
        <KindSwitcher route={route} />
      </section>

      <section className="drawer-section">
        <h3 className="t-label">MANIFEST</h3>
        {route.manifest.map(([item, rate]) => (
          <div className="drawer-row" key={item}>
            <ItemIcon item={item} displayName={gamedata.items[item]?.displayName} size={20} />
            <span className="drawer-row-name">{itemLabel(gamedata.items, item)}</span>
            <span className="t-data-12 projected">
              {fmtRate(rate)}
              <span className="unit">/min</span>
            </span>
          </div>
        ))}
      </section>

      {rail && (
        <section className="drawer-section">
          <h3 className="t-label">CONSISTS</h3>
          <div className="drawer-row mono" data-testid="consist-row">
            <span className="drawer-row-name">
              {rail.consists}× ({rail.locos}× LOCO + {rail.cars}× FREIGHT)
            </span>
            <button className="chip" onClick={() => respec({ consists: Math.max(1, rail.consists - 1) })}>
              −
            </button>
            <button className="chip" onClick={() => respec({ consists: rail.consists + 1 })} data-testid="btn-add-consist">
              +
            </button>
            <span className="mono" style={{ fontSize: 10, color: "var(--ink-500)" }}>
              CARS
            </span>
            <button className="chip" onClick={() => respec({ cars: Math.max(1, rail.cars - 1) })}>
              −
            </button>
            <button className="chip" onClick={() => respec({ cars: rail.cars + 1 })}>
              +
            </button>
          </div>
        </section>
      )}
      {truck && (
        <section className="drawer-section">
          <h3 className="t-label">FLEET</h3>
          <div className="drawer-row mono">
            <span className="drawer-row-name">{truck.trucks}× TRUCK</span>
            <button className="chip" onClick={() => respec({ trucks: Math.max(1, truck.trucks - 1) })}>
              −
            </button>
            <button className="chip" onClick={() => respec({ trucks: truck.trucks + 1 })}>
              +
            </button>
          </div>
        </section>
      )}

      {t && (
        <section className="drawer-section">
          <h3 className="t-label">THE MATH</h3>
          <div className="math-block mono" data-testid="math-block">
            <div className="math-row">
              <span>ROUND TRIP</span>
              <span className="math-note">2×{fmtKm(t.effectiveLengthM)}</span>
              <span>{fmtClockS(t.roundTripS)}</span>
            </div>
            <div className="math-row">
              <span>LOAD/UNLOAD</span>
              <span className="math-note">
                {rail ? `${rail.stations.length} stations` : kind === "truck" ? "2 stops" : "takeoff+landing"}
              </span>
              <span>{fmtClockS(t.loadUnloadS)}</span>
            </div>
            {t.headwayS != null && rail && (
              <div className="math-row">
                <span>HEADWAY</span>
                <span className="math-note">
                  penalty{" "}
                  <input
                    type="number"
                    className="mono math-edit"
                    min={0}
                    max={60}
                    value={Math.round(rail.headwayPenalty * 100)}
                    onChange={(e) => respec({ headwayPenalty: Number(e.target.value) / 100 })}
                  />
                  %
                </span>
                <span>{fmtClockS(t.headwayS)}</span>
              </div>
            )}
            <div className="math-row math-total">
              <span>RTT</span>
              <span className="math-note" />
              <span>{fmtClockS(t.rttS)}</span>
            </div>
            <div className="math-row">
              <span>THROUGHPUT</span>
              <span className="math-note">{fmtRate(t.perTripItems)}/trip</span>
              <span className="projected">{fmtRate(t.throughputPerMin)}/min</span>
            </div>
            <div className={`math-row ${level !== "ok" ? level : ""}`} data-testid="demand-row">
              <span>DEMAND</span>
              <span className="math-note" />
              <span className="projected">
                {fmtRate(demand)}/min {level === "crit" ? "⚠ SHORT" : level === "warn" ? "UNDER" : level === "idle" ? "IDLE" : "✓"}
              </span>
            </div>
            {t.batteriesPerMin != null && (
              <div className="math-row">
                <span>BATTERIES</span>
                <span className="math-note">{drone?.batteriesPerTrip ?? 0}/trip</span>
                <span className="projected">{fmtRate(t.batteriesPerMin)}/min</span>
              </div>
            )}
            {t.fuelItem && (
              <div className="math-row">
                <span>FUEL</span>
                <span className="math-note">{itemLabel(gamedata.items, t.fuelItem)}</span>
                <span className="math-note">solver-sourced later</span>
              </div>
            )}
          </div>
          {short && rail && (
            <button
              className="chip warn"
              style={{ marginTop: 8 }}
              onClick={() => respec({ consists: rail.consists + 1 })}
              data-testid="btn-suggestion"
            >
              +1 CONSIST → {fmtRate((t.throughputPerMin / rail.consists) * (rail.consists + 1))}/min ✓
            </button>
          )}
        </section>
      )}

      {t && (
        <section className="drawer-section">
          <h3 className="t-label">THE ANSWER</h3>
          <TrainAnswerBlock
            answer={trainAnswerFromMath(
              t,
              rail ? rail.consists : truck ? truck.trucks : 1,
              demand,
            )}
            ctx={{
              kind,
              from: srcFactory?.name ?? "?",
              to: dstFactory?.name ?? "?",
              item:
                gamedata.items[route.manifest[0]?.[0] ?? ""]?.displayName ??
                route.manifest[0]?.[0] ??
                "cargo",
            }}
          />
        </section>
      )}

      {rail && (
        <section className="drawer-section">
          <h3 className="t-label">STATIONS</h3>
          {rail.stations.map((st, i) => (
            <div className="drawer-row" key={i}>
              <span className="drawer-row-name mono">{st.name}</span>
              <span className="mono" style={{ fontSize: 11 }}>
                dwell{" "}
                <input
                  type="number"
                  className="mono math-edit"
                  min={5}
                  max={120}
                  value={st.dwellS}
                  onChange={(e) => {
                    const stations = rail.stations.map((x, j) =>
                      j === i ? { ...x, dwellS: Number(e.target.value) } : x,
                    );
                    respec({ stations });
                  }}
                />
                s
              </span>
            </div>
          ))}
        </section>
      )}

      <footer className="drawer-footer">
        <button
          className="btn btn-ghost"
          onClick={() => {
            setSelection(null);
            void dispatch([{ type: "delete_route", id: route.id }]);
          }}
        >
          DELETE ROUTE
        </button>
      </footer>
    </aside>
  );
}
