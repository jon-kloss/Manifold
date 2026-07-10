// Add a boundary port: item picker. Output ports carry the target slider;
// input ports normally arrive via node claims (ceiling attached), but manual
// creation is allowed — unconstrained until bound.

import { useMemo, useState } from "react";
import { useStore } from "../state/store";
import type { Id, PortDirection } from "../state/types";

export default function AddPortMenu({
  direction,
  factoryId,
  onClose,
}: {
  direction: PortDirection;
  factoryId: Id;
  onClose: () => void;
}) {
  const gamedata = useStore((s) => s.gamedata);
  const plan = useStore((s) => s.plan);
  const dispatch = useStore((s) => s.dispatch);
  const [query, setQuery] = useState("");

  const items = useMemo(() => {
    const q = query.toLowerCase();
    return Object.values(gamedata.items)
      .filter((i) => !q || i.displayName.toLowerCase().includes(q))
      .slice(0, 10);
  }, [gamedata.items, query]);

  const add = (item: string) => {
    const siblings = Object.values(plan.ports).filter(
      (p) => p.factory === factoryId && p.direction === direction,
    ).length;
    void dispatch([
      {
        type: "add_port",
        factory: factoryId,
        direction,
        item,
        rate: 0,
        rateCeiling: null,
        graphPos: { x: direction === "in" ? 0 : 1280, y: 80 + siblings * 128 },
      },
    ]);
    onClose();
  };

  return (
    <div className="addgroup-menu" style={{ left: "50%", top: 60, transform: "translateX(-50%)" }}>
      <input
        autoFocus
        placeholder={`${direction === "in" ? "Input" : "Output"} port — item…`}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Enter" && items[0]) add(items[0].className);
        }}
      />
      <div className="addgroup-list">
        {items.map((i) => (
          <button key={i.className} className="addgroup-item" onClick={() => add(i.className)}>
            <div className="icon-ph s20" />
            <span>{i.displayName}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
