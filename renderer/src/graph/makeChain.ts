// "MAKE FROM RESOURCES" planner — pure, no store/plan access.
//
// Given a factory's assigned raw inputs, work out which items are FULLY makeable
// from them (every leaf of the recipe tree is one of those raws) and expand a
// chosen target into the machine groups + belts that build it. The caller turns
// the result into add_group / add_edge / add_port commands wired to the existing
// input ports.
//
// Scope (MVP, per product decision "only fully-makeable items"): standard
// recipes plus unlocked alternates, manufacturer machines only, solid items only
// (belts, not pipes). One machine group per produced item, sized to its total
// demand across the whole tree (shared intermediates are summed, not duplicated).

import type { GameData, GameRecipe } from "../state/types";
import { POWER_ITEM } from "../state/types";
import { minBeltTier } from "./logistics";

const isManufacturer = (g: GameData, machineClass: string) =>
  g.machines[machineClass]?.kind === "manufacturer";

/** Only solids ride belts; a recipe touching a fluid/gas can't be auto-built
 *  here (pipes are out of scope), so it is excluded. Unknown form defaults to
 *  solid so a sparse catalog doesn't over-exclude. */
const isBeltable = (g: GameData, item: string) => {
  const f = (g.items[item]?.form ?? "").toLowerCase();
  return !f.includes("liquid") && !f.includes("gas");
};

/** Manufacturer recipes usable in the auto-builder: standard or unlocked-alt,
 *  no power pseudo-item, every ingredient AND product belt-carriable. */
function eligibleRecipes(g: GameData, unlocked: ReadonlySet<string>): GameRecipe[] {
  return Object.values(g.recipes).filter(
    (r) =>
      (!r.alternate || unlocked.has(r.className)) &&
      r.producedIn.some((m) => isManufacturer(g, m)) &&
      r.products.every(([p]) => p !== POWER_ITEM && isBeltable(g, p)) &&
      r.ingredients.every(([i]) => isBeltable(g, i)),
  );
}

/** Deterministic recipe preference: standard before alternate, then fewest
 *  ingredients, then class name. */
const preferRecipe = (a: GameRecipe, b: GameRecipe) =>
  Number(a.alternate) - Number(b.alternate) ||
  a.ingredients.length - b.ingredients.length ||
  a.className.localeCompare(b.className);

/** Reachability closure from the available raws: an item is reachable once some
 *  eligible recipe has ALL its ingredients reachable. `chosen` records the
 *  recipe that first reached each produced item — its ingredients are always
 *  reachable earlier, so the chosen-graph is a DAG (no cycles, terminates). */
function resolve(
  g: GameData,
  unlocked: ReadonlySet<string>,
  available: ReadonlySet<string>,
): { reachable: Set<string>; chosen: Map<string, GameRecipe> } {
  const recipes = [...eligibleRecipes(g, unlocked)].sort(preferRecipe);
  const reachable = new Set(available);
  const chosen = new Map<string, GameRecipe>();
  let changed = true;
  while (changed) {
    changed = false;
    for (const r of recipes) {
      if (!r.ingredients.every(([i]) => reachable.has(i))) continue;
      for (const [p] of r.products) {
        if (available.has(p)) continue; // raws stay leaves
        if (!chosen.has(p)) chosen.set(p, r);
        if (!reachable.has(p)) {
          reachable.add(p);
          changed = true;
        }
      }
    }
  }
  return { reachable, chosen };
}

/** Items fully makeable from `available` (excludes the raws themselves), sorted
 *  by display name. */
export function makeableItems(
  g: GameData,
  unlocked: ReadonlySet<string>,
  available: ReadonlySet<string>,
): string[] {
  const { chosen } = resolve(g, unlocked, available);
  return [...chosen.keys()].sort((a, b) =>
    (g.items[a]?.displayName ?? a).localeCompare(g.items[b]?.displayName ?? b),
  );
}

/** Output rate per machine at clock 1.0 for `item` from `r` (/min). */
const perMachineOut = (r: GameRecipe, item: string): number => {
  const qty = r.products.find(([p]) => p === item)?.[1] ?? 0;
  return r.durationS > 0 ? (qty * 60) / r.durationS : 0;
};

export interface ChainGroup {
  item: string;
  machine: string;
  recipe: string;
  count: number;
  clock: number;
  /** topological column (0 = first stage off the raws), for layout. */
  depth: number;
}

export interface ChainBelt {
  /** source item — a raw (wire to its input port) or a produced item (its group). */
  fromItem: string;
  fromRaw: boolean;
  /** destination — a produced item's group, or "OUT" for the final output port. */
  toItem: string;
  item: string;
  rate: number;
  tier: number;
}

export interface ChainPlan {
  target: string;
  rate: number;
  groups: ChainGroup[];
  belts: ChainBelt[];
  /** raws actually consumed (need an input port wired). */
  rawsUsed: string[];
}

/** Expand `target` at `rate`/min into groups + belts. Returns null when the
 *  target isn't fully makeable from `available` (shouldn't happen for items the
 *  picker offered). Shared intermediates are summed into one right-sized group. */
export function planChain(
  g: GameData,
  unlocked: ReadonlySet<string>,
  available: ReadonlySet<string>,
  target: string,
  rate: number,
): ChainPlan | null {
  const { chosen } = resolve(g, unlocked, available);
  if (!chosen.has(target) || rate <= 0) return null;

  // 1. accumulate total demand per item across the whole tree (diamonds summed).
  const demand = new Map<string, number>();
  const addDemand = (item: string, r: number) => {
    demand.set(item, (demand.get(item) ?? 0) + r);
    if (available.has(item)) return; // raw leaf — no further expansion
    const recipe = chosen.get(item);
    if (!recipe) return;
    const out = perMachineOut(recipe, item);
    if (out <= 0) return;
    for (const [ing, qty] of recipe.ingredients) {
      addDemand(ing, r * (qty / (recipe.products.find(([p]) => p === item)?.[1] ?? qty)));
    }
  };
  addDemand(target, rate);

  // 2. depth per item for column layout (raws at 0).
  const depthOf = new Map<string, number>();
  const depth = (item: string): number => {
    if (available.has(item)) return 0;
    if (depthOf.has(item)) return depthOf.get(item)!;
    const recipe = chosen.get(item);
    if (!recipe) return 0;
    depthOf.set(item, 0); // guard (DAG, but be safe)
    const d = 1 + Math.max(0, ...recipe.ingredients.map(([i]) => depth(i)));
    depthOf.set(item, d);
    return d;
  };

  // 3. one group per produced item, sized to its demand.
  const groups: ChainGroup[] = [];
  for (const [item, need] of demand) {
    if (available.has(item)) continue;
    const recipe = chosen.get(item);
    if (!recipe) return null;
    const machine = recipe.producedIn.find((m) => isManufacturer(g, m));
    if (!machine) return null;
    const per = perMachineOut(recipe, item);
    if (per <= 0) return null;
    const count = Math.max(1, Math.ceil(need / per));
    const clock = need / (count * per);
    groups.push({ item, machine, recipe: recipe.className, count, clock, depth: depth(item) });
  }

  // 4. belts: each produced item's ingredient links + the final output belt.
  const belts: ChainBelt[] = [];
  const rawsUsed = new Set<string>();
  for (const gr of groups) {
    const recipe = g.recipes[gr.recipe];
    const outQty = recipe.products.find(([p]) => p === gr.item)?.[1] ?? 1;
    const need = demand.get(gr.item) ?? 0;
    for (const [ing, qty] of recipe.ingredients) {
      const beltRate = need * (qty / outQty);
      const fromRaw = available.has(ing);
      if (fromRaw) rawsUsed.add(ing);
      belts.push({ fromItem: ing, fromRaw, toItem: gr.item, item: ing, rate: beltRate, tier: minBeltTier(beltRate) });
    }
  }
  // final output belt: target group → OUT port.
  belts.push({ fromItem: target, fromRaw: false, toItem: "OUT", item: target, rate, tier: minBeltTier(rate) });

  // group ordering: shallow depth first (nice left→right build order).
  groups.sort((a, b) => a.depth - b.depth || a.item.localeCompare(b.item));
  return { target, rate, groups, belts, rawsUsed: [...rawsUsed] };
}
