// Seed a demo plan through the dev-bridge (same command surface as the UI):
// the Modular Frame factory at 2/min. Usage: node seed.mjs [bridge-url]
const base = process.argv[2] ?? "http://localhost:8791";

async function edit(cmds) {
  const res = await fetch(`${base}/api/edit`, { method: "POST", body: JSON.stringify(cmds) });
  if (!res.ok) throw new Error(`${res.status}: ${await res.text()}`);
  return res.json();
}

const f = await edit([
  { type: "create_factory", name: "MODULAR WORKS", position: { x: -1400, y: 2400 }, region: "GRASS FIELDS" },
]);
const fid = f.created[0];

await edit([{ type: "claim_node", factory: fid, node: "bp_resourcenode496", extractor: "Build_MinerMk2_C", clock: 1.0 }]);

const inP = (
  await edit([
    { type: "add_port", factory: fid, direction: "in", item: "Desc_OreIron_C", rate: 0, rateCeiling: 120, graphPos: { x: 0, y: 260 } },
  ])
).created[0];
const outP = (
  await edit([
    { type: "add_port", factory: fid, direction: "out", item: "Desc_ModularFrame_C", rate: 0, rateCeiling: null, graphPos: { x: 1580, y: 260 } },
  ])
).created[0];

const group = async (machine, recipe, x, y) =>
  (
    await edit([
      { type: "add_group", factory: fid, machine, recipe, count: 1, clock: 1.0, graphPos: { x, y }, floor: 0 },
    ])
  ).created[0];

const smelt = await group("Build_SmelterMk1_C", "Recipe_IngotIron_C", 288, 256);
const rods = await group("Build_ConstructorMk1_C", "Recipe_IronRod_C", 608, 96);
const plates = await group("Build_ConstructorMk1_C", "Recipe_IronPlate_C", 608, 416);
const screws = await group("Build_ConstructorMk1_C", "Recipe_Screw_C", 1088, 64);
const rip = await group("Build_AssemblerMk1_C", "Recipe_IronPlateReinforced_C", 1184, 416);
const mf = await group("Build_AssemblerMk1_C", "Recipe_ModularFrame_C", 1312, 176);

const G = (id) => ({ kind: "group", id });
const P = (id) => ({ kind: "port", id });
const J = (id) => ({ kind: "junction", id });
const belt = (from, to, item, tier) => edit([{ type: "add_edge", factory: fid, from, to, item, tier }]);

// the rod run fans out through an explicit splitter, like a real build
const split = (
  await edit([
    { type: "add_junction", factory: fid, kind: "splitter", graphPos: { x: 912, y: 128 }, floor: 0 },
  ])
).created[0];

await belt(P(inP), G(smelt), "Desc_OreIron_C", 3);
await belt(G(smelt), G(rods), "Desc_IronIngot_C", 2);
await belt(G(smelt), G(plates), "Desc_IronIngot_C", 2);
await belt(G(rods), J(split), "Desc_IronRod_C", 2);
await belt(J(split), G(screws), "Desc_IronRod_C", 1);
await belt(J(split), G(mf), "Desc_IronRod_C", 1);
await belt(G(plates), G(rip), "Desc_IronPlate_C", 1);
await belt(G(screws), G(rip), "Desc_IronScrew_C", 1);
await belt(G(rip), G(mf), "Desc_IronPlateReinforced_C", 1);
await belt(G(mf), P(outP), "Desc_ModularFrame_C", 1);

// a second factory pin — Phase 2 wires it into the empire below
const basinId = (
  await edit([
    { type: "create_factory", name: "COPPER BASIN", position: { x: -900, y: 1150 }, region: "GRASS FIELDS" },
  ])
).created[0];

// vertical factory: screws + RIP on floor 1 (belts to them become lifts)
await edit([{ type: "set_group_floor", id: screws, floor: 1 }]);
await edit([{ type: "set_group_floor", id: rip, floor: 1 }]);

// target: 2 modular frames/min → screw belt Mk.1 runs at 36/60 = 60%
const r = await edit([{ type: "set_port_rate", id: outP, rate: 2.0 }]);
console.log("seeded. power:", r.derived.factories[fid].totalPowerMw.toFixed(1), "MW");

// ---- Phase 2 empire: coal power grid + an inter-factory belt route ----
const plant = (
  await edit([
    { type: "create_factory", name: "COAL PLANT 01", position: { x: 180, y: 1050 }, region: "GRASS FIELDS" },
  ])
).created[0];
await edit([{ type: "claim_node", factory: plant, node: "bp_resourcenode600", extractor: "Build_MinerMk2_C", clock: 1.0 }]);
const coalIn = (
  await edit([
    { type: "add_port", factory: plant, direction: "in", item: "Desc_Coal_C", rate: 0, rateCeiling: 120, graphPos: { x: 0, y: 100 } },
  ])
).created[0];
const mwOut = (
  await edit([
    { type: "add_port", factory: plant, direction: "out", item: "__PowerMW", rate: 0, rateCeiling: null, graphPos: { x: 900, y: 100 } },
  ])
).created[0];
const gens = (
  await edit([
    {
      type: "add_group", factory: plant, machine: "Build_GeneratorCoal_C",
      recipe: "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C", count: 1, clock: 1.0,
      graphPos: { x: 420, y: 64 }, floor: 0,
    },
  ])
).created[0];
await edit([{ type: "add_edge", factory: plant, from: P(coalIn), to: G(gens), item: "Desc_Coal_C", tier: 3 }]);
await edit([{ type: "add_edge", factory: plant, from: G(gens), to: P(mwOut), item: "__PowerMW", tier: 6 }]);
await edit([{ type: "set_port_rate", id: mwOut, rate: 150 }]);

// the copper basin takes the frames — a live belt route across the map
const framesIn = (
  await edit([
    { type: "add_port", factory: basinId, direction: "in", item: "Desc_ModularFrame_C", rate: 2, rateCeiling: null, graphPos: { x: 0, y: 100 } },
  ])
).created[0];
await edit([
  {
    type: "add_route", kind: { kind: "belt", tier: 2 }, from: outP, to: framesIn,
    path: [ { x: -1400, y: 2400 }, { x: -900, y: 1150 } ],
  },
]);

// one grid: plant ⚡ works, plant ⚡ basin
await edit([
  { type: "add_route", kind: { kind: "power" }, from: plant, to: fid, path: [ { x: 180, y: 1050 }, { x: -1400, y: 2400 } ] },
]);
const rr = await edit([
  { type: "add_route", kind: { kind: "power" }, from: plant, to: basinId, path: [ { x: 180, y: 1050 }, { x: -900, y: 1150 } ] },
]);
const grid = rr.derived.circuits[0];
console.log(`grid: ${grid.name} · ${grid.demandMw.toFixed(1)}/${grid.generationMw.toFixed(1)} MW across ${grid.members.length} factories`);
