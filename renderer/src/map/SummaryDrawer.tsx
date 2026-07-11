// Factory summary drawer (mock 2b): 380px, slides over the map's right edge.
// Header → OUTPUTS → INPUTS (claims/ceilings — routes arrive in Phase 2) →
// POWER DRAW → footer OPEN FACTORY ⏎.

import { useState } from "react";
import { useStore } from "../state/store";
import { fmtPower, fmtRate } from "../lib/format";
import type { Factory } from "../state/types";

const STATUS_GLYPH = { planned: "◇", under_construction: "◈", built: "◆" } as const;

export default function SummaryDrawer({ factory }: { factory: Factory }) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const setView = useStore((s) => s.setView);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);
  const [editingName, setEditingName] = useState(false);

  const df = derived.factories[factory.id];
  const ports = factory.ports.map((id) => plan.ports[id]).filter(Boolean);
  const outputs = ports.filter((p) => p.direction === "out");
  const inputs = ports.filter((p) => p.direction === "in");
  const projected = factory.status === "planned" ? "projected" : "";

  return (
    <aside className="drawer summary-drawer" data-testid="summary-drawer">
      <header className="drawer-header">
        <div className="icon-ph s40" />
        <div className="drawer-title-block">
          {editingName ? (
            <input
              autoFocus
              defaultValue={factory.name}
              className="drawer-name-input t-title"
              onBlur={(e) => {
                setEditingName(false);
                const name = e.currentTarget.value.trim();
                if (name && name !== factory.name)
                  void dispatch([{ type: "rename_factory", id: factory.id, name }]);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") setEditingName(false);
              }}
            />
          ) : (
            <button className="drawer-name t-title" onClick={() => setEditingName(true)} title="Rename">
              {factory.name.toUpperCase()}
            </button>
          )}
          <div className="mono drawer-sub">
            {factory.region.toUpperCase()} · {factory.groups.length} GROUPS · {factory.nodeClaims.length} NODES
          </div>
        </div>
        <span className={`chip ${factory.status === "planned" ? "planned" : ""}`}>
          {STATUS_GLYPH[factory.status]} {factory.status.replace("_", " ").toUpperCase()}
        </span>
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      <section className="drawer-section">
        <h3 className="t-label">SITE</h3>
        <div className="drawer-row">
          <span className="drawer-row-name">Elevation</span>
          <input
            type="number"
            className="mono"
            style={{ width: 72, height: 24, textAlign: "right" }}
            key={factory.id}
            defaultValue={Math.round(factory.position.z ?? 0)}
            disabled={factory.status !== "planned"}
            data-testid="factory-elevation"
            onBlur={(e) => {
              const z = Number(e.currentTarget.value);
              if (!Number.isFinite(z) || z === (factory.position.z ?? 0)) return;
              void dispatch([
                {
                  type: "move_factory_pin",
                  id: factory.id,
                  position: { x: factory.position.x, y: factory.position.y, z },
                },
              ]);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
          />
          <span className="unit mono">m</span>
        </div>
      </section>

      <section className="drawer-section">
        <h3 className="t-label">THEME</h3>
        <div className="drawer-row">
          <span className="drawer-row-name">Style guide</span>
          <select
            className="mono"
            style={{ height: 24, maxWidth: 160 }}
            value={factory.styleGuide ?? ""}
            onChange={(e) =>
              void dispatch([
                { type: "set_factory_theme", factory: factory.id, styleGuide: e.target.value || null },
              ])
            }
            data-testid="theme-select"
          >
            <option value="">— none —</option>
            {Object.values(plan.styleGuides).map((g) => (
              <option key={g.id} value={g.id}>
                {g.name}
              </option>
            ))}
          </select>
          <button
            className="chip"
            title="Image→style guide needs a model key (AI OFFLINE) — this starts a manual guide"
            onClick={() => {
              const n = Object.keys(plan.styleGuides).length + 1;
              void dispatch([
                {
                  type: "create_style_guide",
                  guide: {
                    id: "",
                    name: `GUIDE ${n}`,
                    palette: [
                      ["FICSIT Foundation 8m", 0.6],
                      ["Concrete", 0.4],
                    ],
                    massing: "Terraced decks stepping with the terrain.",
                    techniques: ["Belt walls between decks", "Lifts in service towers"],
                    sequence: ["Foundations", "Load-bearing frames", "Machines", "Belting", "Facade"],
                    sourceNote: "MANUAL — aesthetic inference needs a model key (AI OFFLINE)",
                  },
                },
              ]);
            }}
            data-testid="btn-new-guide"
          >
            + NEW GUIDE
          </button>
        </div>
      </section>

      <section className="drawer-section">
        <h3 className="t-label">OUTPUTS</h3>
        {outputs.length === 0 && <div className="drawer-empty">No output ports yet — open the factory to add them.</div>}
        {outputs.map((p) => (
          <div className="drawer-row" key={p.id}>
            <div className="icon-ph s20" />
            <span className="drawer-row-name">{gamedata.items[p.item]?.displayName ?? p.item}</span>
            <span className={`t-data-12 ${projected}`}>
              {fmtRate(df?.ports[p.id] ?? p.rate)}
              <span className="unit">/min</span>
            </span>
          </div>
        ))}
      </section>

      <section className="drawer-section">
        <h3 className="t-label">INPUTS</h3>
        {inputs.length === 0 && <div className="drawer-empty">No inputs — claim a node, then wire it inside.</div>}
        {inputs.map((p) => {
          const used = df?.ports[p.id] ?? 0;
          const ceiling = p.rateCeiling;
          const frac = ceiling ? Math.min(1, used / ceiling) : 0;
          return (
            <div className="drawer-row" key={p.id}>
              <div className="icon-ph s20" />
              <span className="drawer-row-name">{gamedata.items[p.item]?.displayName ?? p.item}</span>
              {ceiling != null && (
                <span className="minibar" aria-hidden>
                  <span
                    className={frac >= 0.95 ? "crit" : frac >= 0.7 ? "warn" : ""}
                    style={{ width: `${frac * 100}%` }}
                  />
                </span>
              )}
              <span className={`t-data-12 ${projected}`}>
                {fmtRate(used)}
                {ceiling != null && <span className="unit">/{fmtRate(ceiling)}</span>}
              </span>
            </div>
          );
        })}
      </section>

      <section className="drawer-section">
        <h3 className="t-label">POWER DRAW</h3>
        <div className="drawer-row">
          <span className="drawer-row-name">Machines at planned clocks</span>
          <span className={`t-data-12 ${projected}`}>{fmtPower(df?.totalPowerMw ?? 0)}</span>
        </div>
      </section>

      <footer className="drawer-footer">
        <button
          className="btn btn-primary"
          style={{ flex: 1, height: 34 }}
          onClick={() => setView({ mode: "factory", factoryId: factory.id })}
          data-testid="btn-open-factory"
        >
          OPEN FACTORY ⏎
        </button>
        {factory.status === "planned" && (
          <button
            className="btn btn-ghost"
            onClick={() => {
              setSelection(null);
              void dispatch([{ type: "delete_factory", id: factory.id }]);
            }}
          >
            DELETE
          </button>
        )}
      </footer>
    </aside>
  );
}
