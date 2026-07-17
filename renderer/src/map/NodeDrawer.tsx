// Resource node drawer: purity/region/claims + the claim flow (extractor pick →
// claim_node + boundary input port with the extraction ceiling, one undo step).

import { useState } from "react";
import { useStore } from "../state/store";
import { extractionRate, EXTRACTORS } from "./maputil";
import { fmtRate, itemLabel } from "../lib/format";
import type { WorldNode } from "../state/types";
import ItemIcon from "../lib/ItemIcon";

export default function NodeDrawer({ node }: { node: WorldNode }) {
  const plan = useStore((s) => s.plan);
  const world = useStore((s) => s.world);
  const gamedata = useStore((s) => s.gamedata);
  const derived = useStore((s) => s.derived);
  const setSelection = useStore((s) => s.setSelection);
  const dispatch = useStore((s) => s.dispatch);

  const factories = Object.values(plan.factories);
  const [factoryId, setFactoryId] = useState(factories[0]?.id ?? "");
  const [extractor, setExtractor] = useState(EXTRACTORS[0]);

  // save-only nodes carry item:"" — degrade to a readable title, never blank.
  const item = itemLabel(gamedata.items, node.item) || "RESOURCE NODE";
  const region = world.regions.find((r) => r.id === node.region)?.name ?? node.region;
  const claims = Object.values(plan.nodeClaims).filter((c) => c.node === node.id);
  const conflict = derived.nodes[node.id]?.conflict ?? false;
  // W2b-C: plan-local position correction (snapshot ⊕ override). The bundled
  // catalog coordinate is the honest "was"; save-only nodes have no catalog row.
  const override = plan.nodeOverrides[node.id];
  const catalogNode = world.nodes.find((n) => n.id === node.id);
  const saveOnly = node.id.startsWith("save:");
  const rate = extractionRate(gamedata.machines[extractor], node.purity, 1.0);

  const claim = () => {
    if (!factoryId) return;
    const portCount = Object.values(plan.ports).filter((p) => p.factory === factoryId && p.direction === "in").length;
    void dispatch([
      { type: "claim_node", factory: factoryId, node: node.id, extractor, clock: 1.0 },
      {
        type: "add_port",
        factory: factoryId,
        direction: "in",
        item: node.item,
        rate: 0,
        rateCeiling: rate,
        graphPos: { x: 0, y: 80 + portCount * 120 },
      },
    ]);
  };

  // Reassign a claim to another factory. Claiming afresh for a second factory
  // would STACK claims and trip the (intentional) double-book conflict; a move
  // instead releases the old claim + its unbound input port and re-creates both
  // on the target — one undo step, ending exactly as a fresh claim there would.
  const moveClaim = (c: (typeof claims)[number], toFactory: string) => {
    if (!toFactory || toFactory === c.factory) return;
    const claimRate = extractionRate(gamedata.machines[c.extractor], node.purity, c.clock);
    const cmds: Parameters<typeof dispatch>[0] = [{ type: "release_node", id: c.id }];
    // Best-effort: retire the boundary input port this claim fed on the old
    // factory. The port isn't linked to the claim, so match conservatively —
    // same item + extraction ceiling, not route-bound, AND not wired into the
    // graph. The unwired guard matters when two indistinguishable claims share
    // this factory: without it we could delete the sibling port that IS belted
    // to a machine and cascade-remove its belts. When only wired/ambiguous
    // matches remain, delete nothing — the claim still moves; the port just
    // becomes an unfed input (honest, non-destructive) the user can prune.
    const portWired = (pid: string) =>
      Object.values(plan.edges).some(
        (e) => (e.from.kind === "port" && e.from.id === pid) || (e.to.kind === "port" && e.to.id === pid),
      );
    const oldPort = Object.values(plan.ports).find(
      (p) =>
        p.factory === c.factory &&
        p.direction === "in" &&
        p.item === node.item &&
        p.boundRoute === null &&
        Math.abs((p.rateCeiling ?? -1) - claimRate) < 0.5 &&
        !portWired(p.id),
    );
    if (oldPort) cmds.push({ type: "delete_port", id: oldPort.id });
    const portCount = Object.values(plan.ports).filter((p) => p.factory === toFactory && p.direction === "in").length;
    cmds.push(
      { type: "claim_node", factory: toFactory, node: node.id, extractor: c.extractor, clock: c.clock },
      {
        type: "add_port",
        factory: toFactory,
        direction: "in",
        item: node.item,
        rate: 0,
        rateCeiling: claimRate,
        graphPos: { x: 0, y: 80 + portCount * 120 },
      },
    );
    void dispatch(cmds);
  };

  return (
    <aside className="drawer summary-drawer" data-testid="node-drawer">
      <header className="drawer-header">
        <ItemIcon item={node.item} displayName={item} size={40} />
        <div className="drawer-title-block">
          <div className="t-title">{item.toUpperCase()}</div>
          <div className="mono drawer-sub">
            {region.toUpperCase()} · {(node.purity || "UNKNOWN").toUpperCase()} NODE · {Math.round(node.z)}M
            {node.zone === "cave" ? " · ▾CAVE" : ""}
          </div>
        </div>
        <button className="drawer-close" onClick={() => setSelection(null)} aria-label="Close">
          ×
        </button>
      </header>

      {override?.pos && (catalogNode || saveOnly) && (
        <section className="drawer-section">
          <h3 className="t-label">POSITION</h3>
          <div className="insp-note" data-testid="node-corrected">
            {saveOnly ? (
              <>Save-only node — reconciled from the save at ({Math.round(node.x)}, {Math.round(node.y)}); it sits on no bundled catalog node.</>
            ) : (
              <>
                Save-corrected — was ({Math.round(catalogNode!.x)}, {Math.round(catalogNode!.y)}) in the bundled catalog, now ({Math.round(node.x)}, {Math.round(node.y)}) from the save. The catalog stays a trusted ambient default; only the plan holds this correction.
              </>
            )}
          </div>
        </section>
      )}

      {node.zone === "cave" && node.entrance && (
        <section className="drawer-section">
          <h3 className="t-label">CAVE ACCESS</h3>
          <div className="insp-note">
            Underground node — belts reach it via the surface entrance{" "}
            {Math.round(Math.hypot(node.entrance.x - node.x, node.entrance.y - node.y))} m away,{" "}
            {Math.round(node.entrance.z - node.z)} m above. Route to the entrance (□ on the map), not the
            overhead position.
          </div>
        </section>
      )}

      <section className="drawer-section">
        <h3 className="t-label">CLAIMS</h3>
        {claims.length === 0 && <div className="drawer-empty">Unclaimed.</div>}
        {claims.map((c) => {
          const others = factories.filter((f) => f.id !== c.factory);
          return (
            <div className="drawer-row" key={c.id}>
              <span className="drawer-row-name">
                {plan.factories[c.factory]?.name ?? "?"} · {gamedata.machines[c.extractor]?.displayName ?? c.extractor}
              </span>
              <span className="t-data-12 projected">
                {fmtRate(extractionRate(gamedata.machines[c.extractor], node.purity, c.clock))}
                <span className="unit">/min</span>
              </span>
              {others.length > 0 && (
                <select
                  aria-label="Move claim to another factory"
                  data-testid="claim-move"
                  value=""
                  onChange={(e) => moveClaim(c, e.target.value)}
                  style={{ height: 22 }}
                  title="Move this claim to another factory"
                >
                  <option value="" disabled>
                    MOVE TO…
                  </option>
                  {others.map((f) => (
                    <option key={f.id} value={f.id}>
                      {f.name}
                    </option>
                  ))}
                </select>
              )}
              <button
                className="btn btn-ghost"
                style={{ height: 22, padding: "0 8px" }}
                onClick={() => void dispatch([{ type: "release_node", id: c.id }])}
              >
                RELEASE
              </button>
            </div>
          );
        })}
        {conflict && (
          <div className="drawer-warn mono">
            ⚠ ×{claims.length} — combined claims exceed this node. Intentional double-booking renders CRIT until
            resolved.
          </div>
        )}
      </section>

      <section className="drawer-section">
        <h3 className="t-label">CLAIM FOR</h3>
        {factories.length === 0 ? (
          <div className="drawer-empty">Place a factory first (N).</div>
        ) : (
          <>
            <div className="drawer-row">
              <select value={factoryId} onChange={(e) => setFactoryId(e.target.value)} style={{ flex: 1, height: 28 }}>
                {factories.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="drawer-row">
              <select value={extractor} onChange={(e) => setExtractor(e.target.value)} style={{ flex: 1, height: 28 }}>
                {EXTRACTORS.filter((c) => gamedata.machines[c]).map((c) => (
                  <option key={c} value={c}>
                    {gamedata.machines[c].displayName}
                  </option>
                ))}
              </select>
              <span className="t-data-12 projected">
                {fmtRate(rate)}
                <span className="unit">/min</span>
              </span>
            </div>
            <button className="btn btn-primary" style={{ width: "100%", marginTop: 8 }} onClick={claim} data-testid="btn-claim">
              CLAIM NODE + ADD INPUT PORT
            </button>
          </>
        )}
      </section>
    </aside>
  );
}
