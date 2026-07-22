// Double-click canvas → add machine group: recipe picker at the click point.

import { useMemo, useRef, useState, type RefObject } from "react";
import { useReactFlow } from "@xyflow/react";
import { useStore } from "../state/store";
import type { Id } from "../state/types";
import ItemIcon from "../lib/ItemIcon";
import { useDismiss } from "../lib/useDismiss";

export default function AddGroupMenu({
  at,
  factoryId,
  floor,
  onClose,
}: {
  at: { x: number; y: number; flowX: number; flowY: number };
  factoryId: Id;
  floor: number;
  onClose: () => void;
  flowRef: RefObject<HTMLDivElement | null>;
}) {
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);
  const { screenToFlowPosition } = useReactFlow();
  const [query, setQuery] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);
  useDismiss(rootRef, onClose); // click-off or Escape drops the picker

  const recipes = useMemo(() => {
    const q = query.toLowerCase();
    // Manufacturers, generators AND extractors: a coal plant is placed like any
    // other machine group (its synthesized burn recipe), and a water extractor
    // likewise (its synthesized extraction recipe). Only machines that carry a
    // recipe surface — node-bound miners/oil pumps have none, so including the
    // extractor kind here exposes the water pump without pulling in miners.
    const placeable = new Set(
      Object.values(gamedata.machines)
        .filter((m) => m.kind === "manufacturer" || m.kind === "generator" || m.kind === "extractor")
        .map((m) => m.className),
    );
    const rank = (name: string) => {
      const n = name.toLowerCase();
      if (n === q) return 0;
      if (n.startsWith(q)) return 1;
      return 2;
    };
    return Object.values(gamedata.recipes)
      .filter((r) => !r.alternate && r.producedIn.some((m) => placeable.has(m)))
      .filter((r) => !q || r.displayName.toLowerCase().includes(q))
      .sort((a, b) => rank(a.displayName) - rank(b.displayName) || a.displayName.length - b.displayName.length);
    // No cap: the whole recipe catalog is offered so you can SCROLL the
    // bounded list to find a machine you can't name. (A .slice left the list
    // too short to overflow, so .addgroup-list never scrolled.)
  }, [gamedata, query]);

  const add = (recipeClass: string) => {
    const r = gamedata.recipes[recipeClass];
    const machine = r.producedIn.find((m) => {
      const kind = gamedata.machines[m]?.kind;
      return kind === "manufacturer" || kind === "generator" || kind === "extractor";
    });
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
          floor,
        },
      ],
      { select: true },
    );
    onClose();
  };

  return (
    <div ref={rootRef} className="addgroup-menu" style={{ left: at.x, top: at.y }}>
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
        {recipes.map((r) => {
          // Burn recipes produce the pseudo power item — tag them with the
          // nameplate MW instead of repeating the generator's name.
          const isPower = r.products?.[0]?.[0] === "__PowerMW";
          const machineCls = r.producedIn[0] ?? "";
          return (
            <button key={r.className} className="addgroup-item" onClick={() => add(r.className)}>
              <ItemIcon item={r.products?.[0]?.[0] ?? ""} displayName={r.displayName} size={20} />
              <span>{r.displayName}</span>
              <span className="mono addgroup-sub">
                {isPower
                  ? `⚡ ${r.products[0][1]} MW`
                  : gamedata.machines[machineCls]?.displayName?.toUpperCase()}
              </span>
              <ItemIcon
                item={machineCls}
                displayName={gamedata.machines[machineCls]?.displayName}
                size={20}
              />
            </button>
          );
        })}
      </div>
    </div>
  );
}
