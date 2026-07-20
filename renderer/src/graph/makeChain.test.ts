import { describe, it, expect } from "vitest";
import type { GameData } from "../state/types";
import { makeableItems, planChain, planRawWiring, powerOptions, sizePowerBank, splitAcrossPorts } from "./makeChain";

// Minimal synthetic catalog: Iron Ore (raw) → Iron Ingot → {Iron Plate, Iron Rod},
// plus Water (fluid) → Fake item to prove fluids are excluded.
const G = {
  items: {
    Desc_OreIron_C: { className: "Desc_OreIron_C", displayName: "Iron Ore", form: "RF_SOLID", stackSize: "" },
    Desc_IronIngot_C: { className: "Desc_IronIngot_C", displayName: "Iron Ingot", form: "RF_SOLID", stackSize: "" },
    Desc_IronPlate_C: { className: "Desc_IronPlate_C", displayName: "Iron Plate", form: "RF_SOLID", stackSize: "" },
    Desc_IronRod_C: { className: "Desc_IronRod_C", displayName: "Iron Rod", form: "RF_SOLID", stackSize: "" },
    Desc_Water_C: { className: "Desc_Water_C", displayName: "Water", form: "RF_LIQUID", stackSize: "" },
    Desc_Wet_C: { className: "Desc_Wet_C", displayName: "Wet Thing", form: "RF_SOLID", stackSize: "" },
  },
  machines: {
    Build_Smelter_C: { className: "Build_Smelter_C", kind: "manufacturer", powerMw: 4 },
    Build_Constructor_C: { className: "Build_Constructor_C", kind: "manufacturer", powerMw: 4 },
  },
  recipes: {
    Recipe_Ingot: { className: "Recipe_Ingot", displayName: "Iron Ingot", durationS: 2, ingredients: [["Desc_OreIron_C", 1]], products: [["Desc_IronIngot_C", 1]], producedIn: ["Build_Smelter_C"], alternate: false },
    Recipe_Plate: { className: "Recipe_Plate", displayName: "Iron Plate", durationS: 6, ingredients: [["Desc_IronIngot_C", 3]], products: [["Desc_IronPlate_C", 2]], producedIn: ["Build_Constructor_C"], alternate: false },
    Recipe_Rod: { className: "Recipe_Rod", displayName: "Iron Rod", durationS: 4, ingredients: [["Desc_IronIngot_C", 1]], products: [["Desc_IronRod_C", 1]], producedIn: ["Build_Constructor_C"], alternate: false },
    // needs a fluid → must never be offered/planned
    Recipe_Wet: { className: "Recipe_Wet", displayName: "Wet Thing", durationS: 2, ingredients: [["Desc_IronIngot_C", 1], ["Desc_Water_C", 1]], products: [["Desc_Wet_C", 1]], producedIn: ["Build_Constructor_C"], alternate: false },
  },
} as unknown as GameData;

const AVAIL = new Set(["Desc_OreIron_C"]);
const NONE = new Set<string>();

describe("makeableItems", () => {
  it("returns exactly the items fully makeable from the raws (fluids excluded)", () => {
    expect(makeableItems(G, NONE, AVAIL)).toEqual(["Desc_IronIngot_C", "Desc_IronPlate_C", "Desc_IronRod_C"]);
  });
  it("nothing is makeable from no inputs", () => {
    expect(makeableItems(G, NONE, NONE)).toEqual([]);
  });
});

describe("planChain", () => {
  it("expands Iron Plate @ 20/min into right-sized ingot + plate groups", () => {
    const plan = planChain(G, NONE, AVAIL, "Desc_IronPlate_C", 20)!;
    expect(plan).not.toBeNull();
    const byItem = Object.fromEntries(plan.groups.map((gr) => [gr.item, gr]));
    // plate: 20/min, one constructor makes 20/min → 1× @ 100%
    expect(byItem.Desc_IronPlate_C.count).toBe(1);
    expect(byItem.Desc_IronPlate_C.clock).toBeCloseTo(1.0);
    // plate needs 3 ingot / 2 plate → 30/min ingot; one smelter makes 30/min → 1×
    expect(byItem.Desc_IronIngot_C.count).toBe(1);
    expect(plan.rawsUsed).toEqual(["Desc_OreIron_C"]);
    // ore belt carries 30/min, final output belt carries 20/min
    const oreBelt = plan.belts.find((b) => b.item === "Desc_OreIron_C")!;
    expect(oreBelt.fromRaw).toBe(true);
    expect(oreBelt.rate).toBeCloseTo(30);
    const out = plan.belts.find((b) => b.toItem === "OUT")!;
    expect(out.item).toBe("Desc_IronPlate_C");
    expect(out.rate).toBeCloseTo(20);
  });

  it("sums a shared intermediate instead of duplicating it (Plate + Rod both need ingot)", () => {
    // build Rod @ 15/min: needs 15/min ingot; and separately Plate would need 30.
    const plan = planChain(G, NONE, AVAIL, "Desc_IronRod_C", 15)!;
    const ingot = plan.groups.find((gr) => gr.item === "Desc_IronIngot_C")!;
    expect(ingot.count).toBe(1); // 15/min ingot → 1 smelter @ 50%
    expect(ingot.clock).toBeCloseTo(0.5);
  });

  it("rejects an un-makeable target", () => {
    expect(planChain(G, NONE, AVAIL, "Desc_Wet_C", 10)).toBeNull();
  });
});

describe("splitAcrossPorts", () => {
  it("fills one port when its headroom covers the demand", () => {
    const pool = [
      { id: "a", left: 60 },
      { id: "b", left: 60 },
    ];
    expect(splitAcrossPorts(pool, 45)).toEqual([{ id: "a", rate: 45 }]);
    expect(pool[0].left).toBeCloseTo(15);
    expect(pool[1].left).toBeCloseTo(60);
  });

  it("overflows onto the sibling port when one node can't feed it (2-node merge)", () => {
    const pool = [
      { id: "a", left: 60 },
      { id: "b", left: 60 },
    ];
    const shares = splitAcrossPorts(pool, 100);
    expect(shares).toEqual([
      { id: "a", rate: 60 },
      { id: "b", rate: 40 },
    ]);
    expect(pool[1].left).toBeCloseTo(20);
  });

  it("draws from remaining headroom across successive belts (shared raw, two consumers)", () => {
    const pool = [
      { id: "a", left: 60 },
      { id: "b", left: 60 },
    ];
    expect(splitAcrossPorts(pool, 50)).toEqual([{ id: "a", rate: 50 }]);
    // second consumer of the same raw: 10 left on a, rest from b
    expect(splitAcrossPorts(pool, 40)).toEqual([
      { id: "a", rate: 10 },
      { id: "b", rate: 30 },
    ]);
  });

  it("treats a ceiling-less port as unlimited", () => {
    const pool = [{ id: "free", left: Infinity }];
    expect(splitAcrossPorts(pool, 500)).toEqual([{ id: "free", rate: 500 }]);
    expect(pool[0].left).toBe(Infinity);
  });

  it("piles float dust / overshoot onto the LAST contributing port — never the first", () => {
    const pool = [
      { id: "a", left: 30 },
      { id: "b", left: 30 },
    ];
    // guard would block this; wiring must not drop the 10 — and the overload
    // must land on b so the UI caps the RIGHT port
    expect(splitAcrossPorts(pool, 70)).toEqual([
      { id: "a", rate: 30 },
      { id: "b", rate: 40 },
    ]);
  });

  it("wires residual demand to the first port when the whole pool is exhausted", () => {
    // reachable through the guard's 1e-6 vs the splitter's 1e-9 tolerance gap:
    // an earlier belt drains the pool exactly, float dust remains on the next.
    const pool = [
      { id: "a", left: 0 },
      { id: "b", left: 0 },
    ];
    expect(splitAcrossPorts(pool, 10)).toEqual([{ id: "a", rate: 10 }]);
  });

  it("skips a zero-headroom port at the head of the pool", () => {
    // port a is fully drawn by existing lines — the demand belongs to b
    const pool = [
      { id: "a", left: 0 },
      { id: "b", left: 60 },
    ];
    expect(splitAcrossPorts(pool, 30)).toEqual([{ id: "b", rate: 30 }]);
    expect(pool[1].left).toBeCloseTo(30);
  });

  it("returns empty for an empty pool (no port to wire)", () => {
    expect(splitAcrossPorts([], 10)).toEqual([]);
  });
});

describe("planRawWiring", () => {
  const share = (id: string, rate: number) => ({ id, rate });
  const cons = (key: string, rate: number) => ({ key, rate });

  it("1 port → 1 consumer: a plain belt, no junctions", () => {
    const w = planRawWiring([share("p1", 45)], [cons("rod", 45)]);
    expect(w.junctions).toEqual([]);
    expect(w.edges).toEqual([{ from: { kind: "port", id: "p1" }, to: { kind: "consumer", key: "rod" }, rate: 45 }]);
  });

  it("2 ports → 1 consumer: one merger (the 2-SAM-node case)", () => {
    const w = planRawWiring([share("p1", 60), share("p2", 40)], [cons("sam", 100)]);
    expect(w.junctions).toEqual([{ key: "merger-0", kind: "merger" }]);
    expect(w.edges).toEqual([
      { from: { kind: "port", id: "p1" }, to: { kind: "junction", key: "merger-0" }, rate: 60 },
      { from: { kind: "port", id: "p2" }, to: { kind: "junction", key: "merger-0" }, rate: 40 },
      { from: { kind: "junction", key: "merger-0" }, to: { kind: "consumer", key: "sam" }, rate: 100 },
    ]);
  });

  it("1 port → 2 consumers: one splitter", () => {
    const w = planRawWiring([share("p1", 90)], [cons("a", 60), cons("b", 30)]);
    expect(w.junctions).toEqual([{ key: "splitter-0", kind: "splitter" }]);
    expect(w.edges).toEqual([
      { from: { kind: "port", id: "p1" }, to: { kind: "junction", key: "splitter-0" }, rate: 90 },
      { from: { kind: "junction", key: "splitter-0" }, to: { kind: "consumer", key: "a" }, rate: 60 },
      { from: { kind: "junction", key: "splitter-0" }, to: { kind: "consumer", key: "b" }, rate: 30 },
    ]);
  });

  it("2 ports → 2 consumers: merger then splitter (manifold)", () => {
    const w = planRawWiring([share("p1", 60), share("p2", 60)], [cons("a", 70), cons("b", 50)]);
    expect(w.junctions.map((j) => j.kind)).toEqual(["merger", "splitter"]);
    const mergerOut = w.edges.find((e) => e.from.kind === "junction" && e.to.kind === "junction")!;
    expect(mergerOut.rate).toBeCloseTo(120);
  });

  it("4 ports → 1 consumer: chained mergers respect the 3-in cap", () => {
    const w = planRawWiring(
      [share("p1", 30), share("p2", 30), share("p3", 30), share("p4", 30)],
      [cons("x", 120)],
    );
    expect(w.junctions.filter((j) => j.kind === "merger")).toHaveLength(2);
    // no merger receives more than 3 inputs
    for (const j of w.junctions) {
      const ins = w.edges.filter((e) => e.to.kind === "junction" && e.to.key === j.key).length;
      expect(ins).toBeLessThanOrEqual(3);
    }
    // conservation: consumer receives the full 120
    const last = w.edges.find((e) => e.to.kind === "consumer")!;
    expect(last.rate).toBeCloseTo(120);
  });

  it("1 port → 4 consumers: chained splitters respect the 3-out cap", () => {
    const w = planRawWiring([share("p1", 100)], [cons("a", 25), cons("b", 25), cons("c", 25), cons("d", 25)]);
    expect(w.junctions.filter((j) => j.kind === "splitter")).toHaveLength(2);
    for (const j of w.junctions) {
      const outs = w.edges.filter((e) => e.from.kind === "junction" && e.from.key === j.key).length;
      expect(outs).toBeLessThanOrEqual(3);
    }
    // every consumer is fed exactly once
    const fed = w.edges.filter((e) => e.to.kind === "consumer").map((e) => (e.to as { key: string }).key);
    expect(fed.sort()).toEqual(["a", "b", "c", "d"]);
  });
});

describe("powerOptions / sizePowerBank", () => {
  // Synthetic burn recipe, shaped like gamedata's synthesis: duration 60s,
  // product = __PowerMW at nameplate, ingredient = fuel/min.
  const GP = {
    items: {
      ...((G as unknown as { items: object }).items as object),
      Desc_Coal_C: { className: "Desc_Coal_C", displayName: "Coal", form: "RF_SOLID", stackSize: "" },
    },
    machines: {
      ...((G as unknown as { machines: object }).machines as object),
      Build_GeneratorCoal_C: { className: "Build_GeneratorCoal_C", displayName: "Coal Generator", kind: "generator", powerMw: 75 },
    },
    recipes: {
      ...((G as unknown as { recipes: object }).recipes as object),
      Recipe_Power_Coal: {
        className: "Recipe_Power_Coal",
        displayName: "Coal Generator — Coal",
        durationS: 60,
        ingredients: [["Desc_Coal_C", 15]],
        products: [["__PowerMW", 75]],
        producedIn: ["Build_GeneratorCoal_C"],
        alternate: false,
      },
    },
  } as unknown as GameData;

  it("offers a burn for a fuel the factory has, none otherwise", () => {
    const opts = powerOptions(GP, new Set(["Desc_Coal_C"]));
    expect(opts).toHaveLength(1);
    expect(opts[0]).toMatchObject({
      recipe: "Recipe_Power_Coal",
      machine: "Build_GeneratorCoal_C",
      fuel: "Desc_Coal_C",
      mwPer: 75,
      fuelPer: 15,
    });
    // iron ore in, coal not assigned → no burn offered
    expect(powerOptions(GP, new Set(["Desc_OreIron_C"]))).toEqual([]);
  });

  it("burn recipes never leak into the item picker or item chains", () => {
    expect(makeableItems(GP, NONE, new Set(["Desc_Coal_C"]))).not.toContain("__PowerMW");
    expect(planChain(GP, NONE, new Set(["Desc_Coal_C"]), "__PowerMW", 10)).toBeNull();
  });

  it("sizes the bank: fewest generators, evenly under-clocked, exact fuel", () => {
    // 180 MW on 75 MW gens → 3 gens at 80% clock, burning 36 coal/min
    const s = sizePowerBank({ mwPer: 75, fuelPer: 15 }, 180);
    expect(s.count).toBe(3);
    expect(s.clock).toBeCloseTo(0.8);
    expect(s.fuelNeed).toBeCloseTo(36);
  });

  it("nameplate target lands exactly on 100% clocks", () => {
    const s = sizePowerBank({ mwPer: 75, fuelPer: 15 }, 150);
    expect(s.count).toBe(2);
    expect(s.clock).toBeCloseTo(1.0);
    expect(s.fuelNeed).toBeCloseTo(30);
  });
});

describe("powerOptions exclusions + clock floor (review hardening)", () => {
  const gen = (over: object) => ({
    className: "Recipe_Power_X",
    displayName: "X",
    durationS: 60,
    ingredients: [["Desc_Coal_C", 15]],
    products: [["__PowerMW", 75]],
    producedIn: ["Build_GeneratorCoal_C"],
    alternate: false,
    ...over,
  });
  const cat = (recipeOver: object, itemsOver: object = {}) =>
    ({
      items: {
        Desc_Coal_C: { className: "Desc_Coal_C", displayName: "Coal", form: "RF_SOLID", stackSize: "" },
        Desc_LiquidFuel_C: { className: "Desc_LiquidFuel_C", displayName: "Fuel", form: "RF_LIQUID", stackSize: "" },
        Desc_Water_C: { className: "Desc_Water_C", displayName: "Water", form: "RF_LIQUID", stackSize: "" },
        ...itemsOver,
      },
      machines: {
        Build_GeneratorCoal_C: { className: "Build_GeneratorCoal_C", displayName: "Coal Generator", kind: "generator", powerMw: 75 },
        Build_Constructor_C: { className: "Build_Constructor_C", displayName: "Constructor", kind: "manufacturer", powerMw: 4 },
      },
      recipes: { Recipe_Power_X: gen(recipeOver) },
    }) as unknown as GameData;

  it("excludes a FLUID fuel even when it's an assigned input (no pipes)", () => {
    const g = cat({ ingredients: [["Desc_LiquidFuel_C", 20]] });
    expect(powerOptions(g, new Set(["Desc_LiquidFuel_C"]))).toEqual([]);
  });

  it("excludes a __PowerMW recipe produced in a NON-generator machine", () => {
    const g = cat({ producedIn: ["Build_Constructor_C"] });
    expect(powerOptions(g, new Set(["Desc_Coal_C"]))).toEqual([]);
  });

  it("offers a solid-fuel + water burn (coal/nuclear), sourced from the fuel, with the coolant surfaced", () => {
    // Pipes are modelled now: a coal generator burns coal AND water. It's
    // offered from the coal input; the water rides as `coolant` so the UI can
    // note it and the built group demands it.
    const g = cat({ ingredients: [["Desc_Coal_C", 15], ["Desc_Water_C", 45]] });
    const opts = powerOptions(g, new Set(["Desc_Coal_C"]));
    expect(opts).toHaveLength(1);
    expect(opts[0].fuel).toBe("Desc_Coal_C");
    expect(opts[0].fuelPer).toBe(15);
    expect(opts[0].coolant).toEqual({ item: "Desc_Water_C", perMin: 45 });
  });

  it("excludes a burn with two solid ingredients (only one solid fuel allowed)", () => {
    const g = cat(
      { ingredients: [["Desc_Coal_C", 15], ["Desc_Compacted_C", 5]] },
      { Desc_Compacted_C: { className: "Desc_Compacted_C", displayName: "Compacted Coal", form: "RF_SOLID", stackSize: "" } },
    );
    expect(powerOptions(g, new Set(["Desc_Coal_C", "Desc_Compacted_C"]))).toEqual([]);
  });

  it("excludes multi-product burns (waste output not modeled)", () => {
    const g = cat({ products: [["__PowerMW", 2500], ["Desc_NuclearWaste_C", 10]] });
    expect(powerOptions(g, new Set(["Desc_Coal_C"]))).toEqual([]);
  });

  it("still offers the plain solid single-fuel burn (control)", () => {
    const g = cat({});
    expect(powerOptions(g, new Set(["Desc_Coal_C"]))).toHaveLength(1);
  });

  it("clock floors at the game's 1% minimum; fuel follows the real clock", () => {
    // 1 MW on a 150 MW nameplate would be 0.67% — impossible in game.
    const s = sizePowerBank({ mwPer: 150, fuelPer: 30 }, 1);
    expect(s.count).toBe(1);
    expect(s.clock).toBeCloseTo(0.01);
    expect(s.fuelNeed).toBeCloseTo(0.3); // 1 gen × 1% × 30/min
  });
});
