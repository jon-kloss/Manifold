// Double-click canvas → add machine group: recipe picker at the click point.

import { useMemo, useState, type RefObject } from "react";
import { useReactFlow } from "@xyflow/react";
import { useStore } from "../state/store";
import type { Id } from "../state/types";

export default function AddGroupMenu({
  at,
  factoryId,
  onClose,
}: {
  at: { x: number; y: number; flowX: number; flowY: number };
  factoryId: Id;
  onClose: () => void;
  flowRef: RefObject<HTMLDivElement | null>;
}) {
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);
  const { screenToFlowPosition } = useReactFlow();
  const [query, setQuery] = useState("");

  const recipes = useMemo(() => {
    const q = query.toLowerCase();
    const manufacturers = new Set(
      Object.values(gamedata.machines)
        .filter((m) => m.kind === "manufacturer")
        .map((m) => m.className),
    );
    const rank = (name: string) => {
      const n = name.toLowerCase();
      if (n === q) return 0;
      if (n.startsWith(q)) return 1;
      return 2;
    };
    return Object.values(gamedata.recipes)
      .filter((r) => !r.alternate && r.producedIn.some((m) => manufacturers.has(m)))
      .filter((r) => !q || r.displayName.toLowerCase().includes(q))
      .sort((a, b) => rank(a.displayName) - rank(b.displayName) || a.displayName.length - b.displayName.length)
      .slice(0, 10);
  }, [gamedata, query]);

  const add = (recipeClass: string) => {
    const r = gamedata.recipes[recipeClass];
    const machine = r.producedIn.find((m) => gamedata.machines[m]?.kind === "manufacturer");
    if (!machine) return;
    const pos = screenToFlowPosition({ x: at.flowX, y: at.flowY });
    void dispatch(
      [
        {
          type: "add_group",
          factory: factoryId,
          machine,
          recipe: recipeClass,
          count: 1,
          clock: 1.0,
          graphPos: { x: Math.round(pos.x / 16) * 16, y: Math.round(pos.y / 16) * 16 },
        },
      ],
      { select: true },
    );
    onClose();
  };

  return (
    <div className="addgroup-menu" style={{ left: at.x, top: at.y }}>
      <input
        autoFocus
        placeholder="Add machine group — recipe…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Enter" && recipes[0]) add(recipes[0].className);
        }}
      />
      <div className="addgroup-list">
        {recipes.map((r) => (
          <button key={r.className} className="addgroup-item" onClick={() => add(r.className)}>
            <div className="icon-ph s20" />
            <span>{r.displayName}</span>
            <span className="mono addgroup-sub">
              {gamedata.machines[r.producedIn[0]]?.displayName?.toUpperCase()}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
