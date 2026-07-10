// Recipe strip (mock 4a, bottom-center, build-menu style tiles). Current =
// orange border; available (standard, unlocked) = steel; alternates render
// locked in Phase 1 — no unlock model until progression lands (DECISIONS.md).

import { useMemo } from "react";
import { useStore } from "../state/store";
import type { MachineGroup } from "../state/types";

export default function RecipeStrip({ group }: { group: MachineGroup }) {
  const gamedata = useStore((s) => s.gamedata);
  const dispatch = useStore((s) => s.dispatch);

  const recipes = useMemo(() => {
    const manufacturers = new Set(
      Object.values(gamedata.machines)
        .filter((m) => m.kind === "manufacturer")
        .map((m) => m.className),
    );
    return Object.values(gamedata.recipes)
      .filter((r) => r.producedIn.some((m) => manufacturers.has(m)))
      .sort((a, b) => Number(a.alternate) - Number(b.alternate) || a.displayName.localeCompare(b.displayName));
  }, [gamedata]);

  const pick = (recipeClass: string) => {
    const r = gamedata.recipes[recipeClass];
    const machine = r.producedIn.find((m) => gamedata.machines[m]?.kind === "manufacturer");
    if (!machine) return;
    void dispatch([{ type: "set_group_recipe", id: group.id, machine, recipe: recipeClass }]);
  };

  return (
    <div className="recipe-strip" data-testid="recipe-strip">
      <div className="recipe-strip-head t-label">
        RECIPES <span className="key-hint">R</span>
      </div>
      <div className="recipe-strip-tiles">
        {recipes.map((r) => {
          const current = r.className === group.recipe;
          const locked = r.alternate;
          return (
            <button
              key={r.className}
              className={`recipe-tile ${current ? "current" : ""} ${locked ? "locked" : ""}`}
              disabled={locked || current}
              onClick={() => pick(r.className)}
              title={r.displayName}
            >
              <div className="icon-ph s28" />
              <span className="recipe-tile-name">{r.displayName}</span>
              <span className="mono recipe-tile-sub">
                {locked ? "NOT UNLOCKED" : gamedata.machines[r.producedIn[0]]?.displayName?.toUpperCase() ?? ""}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
