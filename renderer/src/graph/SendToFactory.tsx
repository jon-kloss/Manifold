// "Send to another factory" — the in-graph entry point for inter-factory
// supply (the map right-drag was the only way before, and undiscoverable from
// a factory's own graph). Launched from an OUT port's inspector, it routes one
// OR MORE of this factory's outputs into a chosen target factory in one go —
// creating the matching IN ports on the target when they don't exist yet, so a
// target can accumulate MULTIPLE inputs (from here and from repeated sends by
// other factories). Reuses the exact OUT→IN, item-matched binding the map
// popover uses; the empire recompute then flows the supply across.

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

export default function SendToFactory({
  sourceFactory,
  initialOutPort,
  onClose,
}: {
  sourceFactory: Id;
  initialOutPort: Id;
  onClose: () => void;
}) {
  const plan = useStore((s) => s.plan);
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);
  const setSelection = useStore((s) => s.setSelection);

  const src = plan.factories[sourceFactory];

  // Other factories, by name — the possible destinations.
  const targets = useMemo(
    () =>
      Object.values(plan.factories)
        .filter((f) => f.id !== sourceFactory)
        .sort((a, b) => a.name.localeCompare(b.name)),
    [plan.factories, sourceFactory],
  );
  const [target, setTarget] = useState<Id | "">(targets[0]?.id ?? "");

  // This factory's OUT ports (skip power — that rides a line, not a belt).
  const outPorts = useMemo(
    () =>
      (src?.ports ?? [])
        .map((id) => plan.ports[id])
        .filter((p): p is NonNullable<typeof p> => !!p && p.direction === "out" && p.item !== POWER_ITEM),
    [src, plan.ports],
  );

  // Multi-select: the launching port starts checked; the user can add more so
  // several outputs go to the same target in one confirm.
  const [checked, setChecked] = useState<Record<Id, boolean>>({ [initialOutPort]: true });
  const toggle = (id: Id) => setChecked((c) => ({ ...c, [id]: !c[id] }));

  const dist = (() => {
    const a = src?.position;
    const b = target ? plan.factories[target]?.position : null;
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

  const chosen = outPorts.filter((p) => checked[p.id] && !p.boundRoute);
  const busyRef = useRef(false);

  const confirm = async () => {
    if (busyRef.current || !target || chosen.length === 0) return;
    const dst = plan.factories[target];
    if (!src || !dst) return;
    busyRef.current = true;
    try {
      const routeIds = await wireSupply(
        plan,
        dispatch,
        src,
        dst,
        chosen.map((p) => p.id),
        kindFor(),
      );
      if (routeIds[0]) setSelection({ kind: "route", id: routeIds[0] });
    } finally {
      busyRef.current = false;
      onClose();
    }
  };

  return (
    <div className="send-modal-backdrop" data-testid="send-to-factory" onClick={onClose}>
      <div className="send-modal" onClick={(e) => e.stopPropagation()}>
        <div className="t-label send-modal-title">SEND OUTPUT → FACTORY</div>

        {targets.length === 0 ? (
          <div className="drawer-empty">
            No other factory to send to yet — create a second factory on the map first.
          </div>
        ) : (
          <>
            <label className="send-row">
              <span className="drawer-row-name">To factory</span>
              <select className="mono" value={target} onChange={(e) => setTarget(e.target.value)} data-testid="send-target">
                {targets.map((f) => (
                  <option key={f.id} value={f.id}>
                    {f.name}
                  </option>
                ))}
              </select>
            </label>

            <div className="send-outputs">
              <div className="drawer-row-name" style={{ marginBottom: 4 }}>
                Outputs to send
              </div>
              {outPorts.map((p) => (
                <label className={`send-cand ${p.boundRoute ? "disabled" : ""}`} key={p.id}>
                  <input
                    type="checkbox"
                    checked={!!checked[p.id] && !p.boundRoute}
                    disabled={!!p.boundRoute}
                    onChange={() => toggle(p.id)}
                  />
                  <span>{itemLabel(gamedata.items, p.item)}</span>
                  <span className="mono send-cand-rate">
                    {p.boundRoute ? "already routed" : `${fmtRate(p.rate)}/min`}
                  </span>
                </label>
              ))}
            </div>

            <label className="send-row">
              <span className="drawer-row-name">Transport</span>
              <select
                className="mono"
                value={transport}
                onChange={(e) => setTransport(e.target.value as typeof transport)}
                data-testid="send-transport"
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
                disabled={!target || chosen.length === 0}
                onClick={() => void confirm()}
                data-testid="send-confirm"
              >
                SEND {chosen.length > 1 ? `${chosen.length} OUTPUTS` : "OUTPUT"}
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
