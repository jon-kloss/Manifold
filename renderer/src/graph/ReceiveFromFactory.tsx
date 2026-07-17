// "Receive from another factory" — the mirror of SendToFactory, launched from
// a target factory's IN port. Pick a SOURCE factory, then choose which of its
// outputs to pull in (the one matching this input is pre-checked). Same
// item-matched OUT→IN binding via wireSupply, so an input that read
// "UNROUTED — SUPPLY ASSUMED" becomes real supply from a chosen factory — and
// the target can pull several inputs (from this source and others).

import { useMemo, useRef, useState } from "react";
import { useStore } from "../state/store";
import {
  beltCapacity,
  DEFAULT_DRONE_SPEC,
  DEFAULT_RAIL_SPEC,
  DEFAULT_TRUCK_SPEC,
  POWER_ITEM,
  type Id,
  type RouteKind,
} from "../state/types";
import { fmtRate, itemLabel } from "../lib/format";
import { wireSupply } from "./interFactorySupply";

export default function ReceiveFromFactory({
  targetFactory,
  initialInPort,
  onClose,
}: {
  targetFactory: Id;
  initialInPort: Id;
  onClose: () => void;
}) {
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);
  const setSelection = useStore((s) => s.setSelection);

  const target = plan.factories[targetFactory];
  const wantItem = plan.ports[initialInPort]?.item ?? null;

  // Unbound, non-power OUT ports on a factory — its available exports.
  const freeOuts = useMemo(
    () => (fid: Id) =>
      (plan.factories[fid]?.ports ?? [])
        .map((id) => plan.ports[id])
        .filter((p): p is NonNullable<typeof p> => !!p && p.direction === "out" && !p.boundRoute && p.item !== POWER_ITEM),
    [plan.factories, plan.ports],
  );

  // Sources: other factories with at least one exportable output. Ones that
  // produce the wanted item sort first so the obvious supplier is the default.
  const sources = useMemo(
    () =>
      Object.values(plan.factories)
        .filter((f) => f.id !== targetFactory && freeOuts(f.id).length > 0)
        .sort((a, b) => {
          const aHas = freeOuts(a.id).some((p) => p.item === wantItem) ? 0 : 1;
          const bHas = freeOuts(b.id).some((p) => p.item === wantItem) ? 0 : 1;
          return aHas - bHas || a.name.localeCompare(b.name);
        }),
    [plan.factories, targetFactory, freeOuts, wantItem],
  );
  const [source, setSource] = useState<Id | "">(sources[0]?.id ?? "");

  const sourceOuts = source ? freeOuts(source) : [];
  // Default-check the output that feeds this input; recomputed when the source
  // changes so the pre-selection always matches the wanted item.
  const [checked, setChecked] = useState<Record<Id, boolean> | null>(null);
  const effectiveChecked = useMemo(() => {
    if (checked) return checked;
    const seed: Record<Id, boolean> = {};
    const match = sourceOuts.find((p) => p.item === wantItem);
    if (match) seed[match.id] = true;
    return seed;
  }, [checked, sourceOuts, wantItem]);
  const toggle = (id: Id) => setChecked({ ...effectiveChecked, [id]: !effectiveChecked[id] });

  const dist = (() => {
    const a = source ? plan.factories[source]?.position : null;
    const b = target?.position;
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  })();
  const [transport, setTransport] = useState<"belt" | "rail" | "truck" | "drone">("belt");
  const [tier, setTier] = useState(3);
  const kindFor = (): RouteKind =>
    transport === "belt"
      ? { kind: "belt", tier }
      : transport === "rail"
        ? { kind: "rail", spec: { ...DEFAULT_RAIL_SPEC } }
        : transport === "truck"
          ? { kind: "truck", spec: { ...DEFAULT_TRUCK_SPEC } }
          : { kind: "drone", spec: { ...DEFAULT_DRONE_SPEC } };

  const chosen = sourceOuts.filter((p) => effectiveChecked[p.id]);
  const busyRef = useRef(false);

  const confirm = async () => {
    if (busyRef.current || !source || chosen.length === 0) return;
    const src = plan.factories[source];
    if (!src || !target) return;
    busyRef.current = true;
    try {
      const routeIds = await wireSupply(
        plan,
        dispatch,
        src,
        target,
        chosen.map((p) => p.id),
        kindFor(),
        initialInPort, // bind the port the user launched from, not a sibling
      );
      if (routeIds[0]) setSelection({ kind: "route", id: routeIds[0] });
    } finally {
      busyRef.current = false;
      onClose();
    }
  };

  return (
    <div className="send-modal-backdrop" data-testid="receive-from-factory" onClick={onClose}>
      <div className="send-modal" onClick={(e) => e.stopPropagation()}>
        <div className="t-label send-modal-title">RECEIVE INPUT ← FACTORY</div>

        {sources.length === 0 ? (
          <div className="drawer-empty">
            No other factory has a free output to pull from yet — add an OUT port on a supplier first.
          </div>
        ) : (
          <>
            <label className="send-row">
              <span className="drawer-row-name">From factory</span>
              <select
                className="mono"
                value={source}
                onChange={(e) => {
                  setSource(e.target.value);
                  setChecked(null); // re-seed the default check for the new source
                }}
                data-testid="receive-source"
              >
                {sources.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.name}
                    {freeOuts(f.id).some((p) => p.item === wantItem) ? " ✓" : ""}
                  </option>
                ))}
              </select>
            </label>

            <div className="send-outputs">
              <div className="drawer-row-name" style={{ marginBottom: 4 }}>
                Outputs to pull
              </div>
              {sourceOuts.length === 0 ? (
                <div className="drawer-empty" style={{ padding: 4 }}>
                  This factory has no free outputs.
                </div>
              ) : (
                sourceOuts.map((p) => (
                  <label className="send-cand" key={p.id}>
                    <input type="checkbox" checked={!!effectiveChecked[p.id]} onChange={() => toggle(p.id)} />
                    <span>
                      {itemLabel(gamedata.items, p.item)}
                      {p.item === wantItem ? " ·" : ""}
                    </span>
                    <span className="mono send-cand-rate">{fmtRate(p.rate)}/min</span>
                  </label>
                ))
              )}
            </div>

            <label className="send-row">
              <span className="drawer-row-name">Transport</span>
              <select
                className="mono"
                value={transport}
                onChange={(e) => setTransport(e.target.value as typeof transport)}
                data-testid="receive-transport"
              >
                <option value="belt">BELT{dist < 800 ? " — suggested" : ""}</option>
                <option value="rail">RAIL{dist >= 800 ? " — suggested" : ""}</option>
                <option value="truck">TRUCK</option>
                <option value="drone">DRONE</option>
              </select>
            </label>
            {transport === "belt" && (
              <label className="send-row">
                <span className="drawer-row-name">Belt tier</span>
                <select className="mono" value={tier} onChange={(e) => setTier(Number(e.target.value))}>
                  {[1, 2, 3, 4, 5, 6].map((t) => (
                    <option key={t} value={t}>
                      MK.{t} — {fmtRate(beltCapacity(t))}/min
                    </option>
                  ))}
                </select>
              </label>
            )}

            <div className="send-actions">
              <button
                className="btn btn-primary"
                disabled={!source || chosen.length === 0}
                onClick={() => void confirm()}
                data-testid="receive-confirm"
              >
                PULL {chosen.length > 1 ? `${chosen.length} INPUTS` : "INPUT"}
              </button>
              <button className="btn btn-ghost" onClick={onClose}>
                CANCEL
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
