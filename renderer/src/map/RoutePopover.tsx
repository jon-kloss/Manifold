// Right-drag route confirmation: pick which OUT→IN port pair the belt binds
// (auto-matched by item; can create the missing IN port on the target), plus
// the belt tier. Nothing mutates until CONFIRM.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../state/store";
import { backend } from "../state/backend";
import {
  beltCapacity,
  DEFAULT_DRONE_SPEC,
  DEFAULT_RAIL_SPEC,
  DEFAULT_TRUCK_SPEC,
  POWER_ITEM,
  type Command,
  type Id,
  type RouteKind,
  type TrainAnswer,
} from "../state/types";
import { fmtRate } from "../lib/format";
import TrainAnswerBlock from "./TrainAnswerBlock";

interface Candidate {
  key: string;
  label: string;
  outPort: Id;
  inPort: Id | null; // null = create on confirm
  item: string;
  power?: boolean; // power line joins the two factories into one circuit
}

export default function RoutePopover({
  fromFactory,
  toFactory,
  onClose,
}: {
  fromFactory: Id;
  toFactory: Id;
  onClose: () => void;
}) {
  const plan = useStore((s) => s.plan);
  const derived = useStore((s) => s.derived);
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);
  const setSelection = useStore((s) => s.setSelection);
  const [tier, setTier] = useState(3);
  // A3.3 default: belt under 800m, rail beyond (drone for trickles is a
  // manual pick — the popover can't know the rate before the route exists)
  const dist = (() => {
    const a = plan.factories[fromFactory]?.position;
    const b = plan.factories[toFactory]?.position;
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  })();
  const [transport, setTransport] = useState<"belt" | "rail" | "truck" | "drone">(dist >= 800 ? "rail" : "belt");
  const kindFor = useCallback(
    (): RouteKind =>
      transport === "belt"
        ? { kind: "belt", tier }
        : transport === "rail"
          ? { kind: "rail", spec: { ...DEFAULT_RAIL_SPEC } }
          : transport === "truck"
            ? { kind: "truck", spec: { ...DEFAULT_TRUCK_SPEC } }
            : { kind: "drone", spec: { ...DEFAULT_DRONE_SPEC } },
    [transport, tier],
  );

  const candidates: Candidate[] = useMemo(() => {
    const src = plan.factories[fromFactory];
    const dst = plan.factories[toFactory];
    if (!src || !dst) return [];
    const out: Candidate[] = [];
    for (const pid of src.ports) {
      const p = plan.ports[pid];
      if (!p || p.direction !== "out" || p.boundRoute) continue;
      if (p.item === POWER_ITEM) continue; // power moves on lines, not belts
      const itemName = gamedata.items[p.item]?.displayName ?? p.item;
      const match = dst.ports
        .map((id) => plan.ports[id])
        .find((q) => q && q.direction === "in" && !q.boundRoute && q.item === p.item);
      if (match) {
        out.push({ key: `${pid}-${match.id}`, label: `${itemName}`, outPort: pid, inPort: match.id, item: p.item });
      } else {
        out.push({
          key: `${pid}-new`,
          label: `${itemName} — new IN port`,
          outPort: pid,
          inPort: null,
          item: p.item,
        });
      }
    }
    // power line: one per factory pair (the backend rejects duplicates)
    const hasLine = Object.values(plan.routes).some(
      (r) =>
        r.kind.kind === "power" &&
        ((r.endpoints[0] === fromFactory && r.endpoints[1] === toFactory) ||
          (r.endpoints[0] === toFactory && r.endpoints[1] === fromFactory)),
    );
    if (!hasLine) {
      out.push({ key: "power", label: "⚡ Power line — join grids", outPort: "", inPort: null, item: "", power: true });
    }
    return out;
  }, [plan, fromFactory, toFactory, gamedata.items]);

  const [picked, setPicked] = useState(0);

  // ---- task #49: the pre-build TRAIN ANSWER. For a rail/truck/drone pick the
  // popover answers "how many trains?" BEFORE the route is committed — read-only
  // (routeCalc creates nothing), from the two pins' distance and the OUT port's
  // demand (or a user-entered target). ----
  const cand = candidates[picked];
  const showTrain = !!cand && !cand.power && transport !== "belt";
  const autoDemand = useMemo(() => {
    if (!cand || cand.power) return 0;
    return derived.factories[fromFactory]?.ports[cand.outPort] ?? plan.ports[cand.outPort]?.rate ?? 0;
  }, [cand, derived, fromFactory, plan.ports]);
  const [target, setTarget] = useState<number | null>(null);
  const demand = target ?? autoDemand;
  const [answer, setAnswer] = useState<TrainAnswer | null>(null);
  useEffect(() => {
    if (!showTrain || !cand) {
      setAnswer(null);
      return;
    }
    let live = true;
    void backend
      .routeCalc(fromFactory, toFactory, kindFor(), demand, cand.item || null)
      .then((a) => live && setAnswer(a))
      .catch(() => live && setAnswer(null));
    return () => {
      live = false;
    };
  }, [showTrain, cand, fromFactory, toFactory, kindFor, demand]);

  // Enter key-repeat (or a double click) must not dispatch add_route twice
  // while the first await is in flight.
  const busyRef = useRef(false);

  const confirm = useCallback(async () => {
    if (busyRef.current) return;
    const c = candidates[picked];
    if (!c) return;
    busyRef.current = true;
    const src = plan.factories[fromFactory]!;
    const dst = plan.factories[toFactory]!;
    const path = [src.position, dst.position];
    // a refused dispatch resolves null (surfaced in the status bar); the
    // finally keeps the popover from sticking open no matter what happens
    try {
      if (c.power) {
        const created = await dispatch([
          { type: "add_route", kind: { kind: "power" }, from: fromFactory, to: toFactory, path },
        ]);
        const id = created?.[0];
        if (id) setSelection({ kind: "route", id });
        return;
      }
      if (c.inPort) {
        const created = await dispatch([
          { type: "add_route", kind: kindFor(), from: c.outPort, to: c.inPort, path },
        ]);
        const id = created?.[0];
        if (id) setSelection({ kind: "route", id });
      } else {
        // create the IN port, then bind — two commands, one undo step
        const inCount = dst.ports.filter((id) => plan.ports[id]?.direction === "in").length;
        const cmds: Command[] = [
          {
            type: "add_port",
            factory: toFactory,
            direction: "in",
            item: c.item,
            rate: 0,
            rateCeiling: null,
            graphPos: { x: 0, y: 80 + inCount * 128 },
          },
        ];
        const created = await dispatch(cmds);
        const newPort = created?.[0];
        // a refused add_port must not attempt the add_route
        if (newPort) {
          const routeIds = await dispatch([
            { type: "add_route", kind: kindFor(), from: c.outPort, to: newPort, path },
          ]);
          const id = routeIds?.[0];
          if (id) setSelection({ kind: "route", id });
        }
      }
    } finally {
      busyRef.current = false;
      onClose();
    }
  }, [candidates, picked, plan, fromFactory, toFactory, kindFor, dispatch, setSelection, onClose]);

  // While the popover is open its Enter/Escape win over MapView's bubble-phase
  // handler (capture + stopPropagation — same precedence pattern as WizardModal
  // and ProposalReview). Deliberately no isEditableTarget guard here: Enter
  // with the transport <select> or a candidate radio focused must confirm, and
  // the popover has no free-text surfaces.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation(); // MapView's ⏎-dive must never fire while we're open
        void confirm();
      } else if (e.key === "Escape") {
        e.stopPropagation(); // ...nor its ESC-deselect
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [confirm, onClose]);

  return (
    <div className="route-popover" data-testid="route-popover">
      <div className="t-label" style={{ marginBottom: 8 }}>
        DRAW ROUTE
      </div>
      {candidates.length === 0 ? (
        <div className="drawer-empty">
          No unbound OUT ports on {plan.factories[fromFactory]?.name}. Add an output port in its factory view
          first.
        </div>
      ) : (
        <>
          {candidates.map((c, i) => (
            <label className="route-cand" key={c.key}>
              <input type="radio" checked={picked === i} onChange={() => setPicked(i)} />
              <span>{c.label}</span>
            </label>
          ))}
          {!candidates[picked]?.power && (
            <div className="drawer-row" style={{ marginTop: 8 }}>
              <span className="drawer-row-name">Transport</span>
              <select
                className="mono"
                style={{ height: 24 }}
                value={transport}
                onChange={(e) => setTransport(e.target.value as typeof transport)}
                data-testid="popover-transport"
              >
                <option value="belt">BELT{dist < 800 ? " — suggested" : ""}</option>
                <option value="rail">RAIL{dist >= 800 ? " — suggested" : ""}</option>
                <option value="truck">TRUCK</option>
                <option value="drone">DRONE</option>
              </select>
            </div>
          )}
          {!candidates[picked]?.power && transport === "belt" && (
            <div className="drawer-row" style={{ marginTop: 8 }}>
              <span className="drawer-row-name">Belt tier</span>
              <select className="mono" style={{ height: 24 }} value={tier} onChange={(e) => setTier(Number(e.target.value))}>
                {[1, 2, 3, 4, 5, 6].map((t) => (
                  <option key={t} value={t}>
                    MK.{t} — {fmtRate(beltCapacity(t))}/min
                  </option>
                ))}
              </select>
            </div>
          )}
          {showTrain && answer && cand && (
            <TrainAnswerBlock
              answer={answer}
              ctx={{
                kind: transport as "rail" | "truck" | "drone",
                from: plan.factories[fromFactory]?.name ?? "?",
                to: plan.factories[toFactory]?.name ?? "?",
                item: gamedata.items[cand.item]?.displayName ?? cand.item,
              }}
              onDemandChange={(r) => setTarget(r)}
            />
          )}
          <div style={{ display: "flex", gap: 8, marginTop: 10 }}>
            <button className="btn btn-primary" style={{ flex: 1 }} onClick={() => void confirm()} data-testid="btn-route-confirm">
              CONFIRM ⏎
            </button>
            <button className="btn btn-ghost" onClick={onClose}>
              CANCEL
            </button>
          </div>
        </>
      )}
    </div>
  );
}
