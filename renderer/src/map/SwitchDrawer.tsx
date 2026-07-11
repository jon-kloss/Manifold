// Priority switch inspector (A2.3): priority P1–P8 (higher sheds first),
// live SHEDS AT threshold from the circuit's brownout math, DELETE.

import { useStore } from "../state/store";
import { fmtPower } from "../lib/format";
import type { PrioritySwitch } from "../state/types";

export default function SwitchDrawer({ sw }: { sw: PrioritySwitch }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);

  const route = plan.routes[sw.route];
  const [aId, bId] = route?.endpoints ?? ["", ""];
  const circuit = derived.circuits.find((c) => c.switches.some((d) => d.id === sw.id));
  const dsw = circuit?.switches.find((d) => d.id === sw.id);

  return (
    <aside className="drawer summary-drawer" data-testid="switch-drawer">
      <header className="drawer-header">
        <div className="icon-ph s40" />
        <div className="drawer-title-block">
          <div className="t-title">PRIORITY SWITCH</div>
          <div className="mono drawer-sub">
            {circuit?.name ?? "UNGRIDDED"} · {plan.factories[aId]?.name.toUpperCase() ?? "?"} ⚡{" "}
            {plan.factories[bId]?.name.toUpperCase() ?? "?"}
          </div>
        </div>
        <span className="chip planned">◇ PLANNED</span>
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      <section className="drawer-section">
        <h3 className="t-label">SHEDDING</h3>
        <div className="drawer-row">
          <span className="drawer-row-name">Priority (higher sheds first)</span>
          <select
            className="mono"
            style={{ height: 24 }}
            value={sw.priority}
            onChange={(e) => void dispatch([{ type: "set_switch_priority", id: sw.id, priority: Number(e.target.value) }])}
            data-testid="switch-priority"
          >
            {[1, 2, 3, 4, 5, 6, 7, 8].map((p) => (
              <option key={p} value={p}>
                P{p}
              </option>
            ))}
          </select>
        </div>
        <div className="drawer-row">
          <span className="drawer-row-name">Sheds at</span>
          <span className="t-data-12 projected">{dsw ? fmtPower(dsw.shedsAtMw) : "—"}</span>
        </div>
        <div className="drawer-row">
          <span className="drawer-row-name">Load shed</span>
          <span className="t-data-12 projected">{dsw ? fmtPower(dsw.downstreamMw) : "—"}</span>
        </div>
        {circuit?.nextShed && <div className="insp-note">Brownout sim: next shed {circuit.nextShed}.</div>}
      </section>

      <footer className="drawer-footer">
        <button
          className="btn btn-ghost"
          onClick={() => {
            setSelection(null);
            void dispatch([{ type: "delete_switch", id: sw.id }]);
          }}
        >
          DELETE SWITCH
        </button>
      </footer>
    </aside>
  );
}
