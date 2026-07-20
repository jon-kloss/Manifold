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

export interface PortShare {
  id: string;
  rate: number;
}

/** Split one belt's raw demand across same-item source ports, consuming each
 *  port's remaining headroom in order — the in-graph merge IS the "merger":
 *  one belt per port that contributes. Mutates `pool[].left` so successive
 *  calls (several consumers of the same raw) keep drawing from what's left.
 *  Any remainder beyond total headroom piles onto the last contributing port:
 *  the capacity guard blocks real overshoot before build, so this only absorbs
 *  float dust — and a knowingly-overloaded port shows honestly as capped. */
export function splitAcrossPorts(pool: { id: string; left: number }[], rate: number): PortShare[] {
  const out: PortShare[] = [];
  let need = rate;
  for (const p of pool) {
    if (need <= 1e-9) break;
    const take = Math.min(p.left, need);
    if (take > 1e-9) {
      out.push({ id: p.id, rate: take });
      p.left -= take;
      need -= take;
    }
  }
  if (need > 1e-9) {
    if (out.length > 0) out[out.length - 1].rate += need;
    else if (pool.length > 0) out.push({ id: pool[0].id, rate: need });
  }
  return out;
}

// ---- MAKE POWER: generator banks from claimed fuel -------------------------
//
// Fuel-burn recipes are synthesized by gamedata (product = __PowerMW at the
// nameplate MW, duration-normalized), so "what power can these raws make?"
// is the same catalog scan as items — just aimed at generators.

export interface PowerOption {
  recipe: string;
  machine: string;
  fuel: string;
  /** nameplate MW per generator (the burn recipe's __PowerMW out-rate). */
  mwPer: number;
  /** fuel items/min per generator at 100% clock. */
  fuelPer: number;
  /** supplemental fluid this generator burns to run (coal/nuclear → water),
   *  or null. The bank is fed from the solid fuel; the fluid rides the built
   *  group's recipe as a demand the solver surfaces until it's piped in. */
  coolant: { item: string; perMin: number } | null;
}

/** Generator burn recipes runnable from `available` raws: one SOLID fuel that
 *  is one of the factory's inputs, plus at most one FLUID supplemental (water,
 *  for coal/nuclear — now that pipes are modelled). Fuel-less generators
 *  (geothermal) and fluid FUELS are still excluded. */
export function powerOptions(g: GameData, available: ReadonlySet<string>): PowerOption[] {
  const out: PowerOption[] = [];
  for (const r of Object.values(g.recipes)) {
    if (r.products.length !== 1 || r.products[0][0] !== POWER_ITEM) continue;
    // Split ingredients into the solid fuel and any fluid supplemental (water).
    const solids = r.ingredients.filter(([i]) => isBeltable(g, i));
    const fluids = r.ingredients.filter(([i]) => !isBeltable(g, i));
    if (solids.length !== 1 || fluids.length > 1) continue;
    const [fuel, fuelQty] = solids[0];
    if (!available.has(fuel)) continue;
    const machine = r.producedIn.find((m) => g.machines[m]?.kind === "generator");
    if (!machine) continue;
    const mwPer = perMachineOut(r, POWER_ITEM);
    const fuelPer = r.durationS > 0 ? (fuelQty * 60) / r.durationS : 0;
    if (mwPer <= 0 || fuelPer <= 0) continue;
    const coolant =
      fluids.length === 1 && r.durationS > 0
        ? { item: fluids[0][0], perMin: (fluids[0][1] * 60) / r.durationS }
        : null;
    out.push({ recipe: r.className, machine, fuel, mwPer, fuelPer, coolant });
  }
  return out.sort((a, b) => b.mwPer - a.mwPer || a.recipe.localeCompare(b.recipe));
}

/** Size a bank for `mw`: fewest generators, evenly under-clocked to hit the
 *  target exactly (same shape planChain uses for item groups). Clock floors
 *  at the game's 1% minimum — a tiny MW ask on a big nameplate builds one
 *  generator at 1% (slightly over target) rather than an impossible clock;
 *  fuel follows the ACTUAL clock so the belts match what really burns. */
export function sizePowerBank(
  opt: Pick<PowerOption, "mwPer" | "fuelPer">,
  mw: number,
): { count: number; clock: number; fuelNeed: number } {
  const count = Math.max(1, Math.ceil(mw / opt.mwPer));
  const clock = Math.max(0.01, mw / (count * opt.mwPer));
  return { count, clock, fuelNeed: count * clock * opt.fuelPer };
}

// ---- raw-supply wiring: real mergers/splitters, like a hand build ----------
//
// A raw drawn from several claims, or feeding several consumers, is wired
// through the SAME junction entities a player would place: a chain of 3-in
// mergers combines the claims into one stream, a chain of 3-out splitters
// fans it out to the consumers. One port → one consumer stays a direct belt.

export interface RawConsumer {
  /** produced item whose group consumes this raw (unique per raw: demand is
   *  summed per item, so each consuming group appears once). */
  key: string;
  rate: number;
}
export type WiringRef =
  | { kind: "port"; id: string }
  | { kind: "junction"; key: string }
  | { kind: "consumer"; key: string };
export interface RawWiring {
  junctions: { key: string; kind: "merger" | "splitter" }[];
  edges: { from: WiringRef; to: WiringRef; rate: number }[];
}

/** Wire `shares` (per-port supply of ONE raw) to `consumers` through game-cap
 *  junctions (merger 3-in/1-out, splitter 1-in/3-out), chaining junctions
 *  manifold-style when a single one is too small. Pure — the caller turns
 *  junction keys into add_junction commands and consumer keys into group ids. */
export function planRawWiring(shares: PortShare[], consumers: RawConsumer[]): RawWiring {
  const junctions: RawWiring["junctions"] = [];
  const edges: RawWiring["edges"] = [];
  let seq = 0;
  const newJunction = (kind: "merger" | "splitter") => {
    const key = `${kind}-${seq++}`;
    junctions.push({ key, kind });
    return key;
  };

  // 1) merge the supply into one logical stream. Chain: the first merger takes
  //    3 ports; each further merger takes the previous stream + 2 more ports.
  let feed: { ref: WiringRef; rate: number };
  if (shares.length === 1) {
    feed = { ref: { kind: "port", id: shares[0].id }, rate: shares[0].rate };
  } else {
    const queue = shares.map((s) => ({ ref: { kind: "port", id: s.id } as WiringRef, rate: s.rate }));
    feed = queue.shift()!;
    while (queue.length) {
      const m = newJunction("merger");
      const ins = [feed, ...queue.splice(0, 2)]; // ≤3 inputs per merger
      for (const i of ins) edges.push({ from: i.ref, to: { kind: "junction", key: m }, rate: i.rate });
      feed = { ref: { kind: "junction", key: m }, rate: ins.reduce((s, x) => s + x.rate, 0) };
    }
  }

  // 2) fan out to the consumers. Chain: each splitter feeds 2 consumers + the
  //    next splitter; the last one feeds up to 3 consumers.
  if (consumers.length === 1) {
    edges.push({ from: feed.ref, to: { kind: "consumer", key: consumers[0].key }, rate: consumers[0].rate });
  } else {
    const rem = [...consumers];
    while (rem.length) {
      const s = newJunction("splitter");
      edges.push({ from: feed.ref, to: { kind: "junction", key: s }, rate: feed.rate });
      const take = rem.splice(0, rem.length <= 3 ? rem.length : 2);
      for (const c of take) edges.push({ from: { kind: "junction", key: s }, to: { kind: "consumer", key: c.key }, rate: c.rate });
      feed = { ref: { kind: "junction", key: s }, rate: rem.reduce((sum, c) => sum + c.rate, 0) };
    }
  }
  return { junctions, edges };
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
