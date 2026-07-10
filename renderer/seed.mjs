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

await edit([{ type: "claim_node", factory: fid, node: "iron-gf-01", extractor: "Build_MinerMk2_C", clock: 1.0 }]);

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
      { type: "add_group", factory: fid, machine, recipe, count: 1, clock: 1.0, graphPos: { x, y } },
    ])
  ).created[0];

const smelt = await group("Build_SmelterMk1_C", "Recipe_IngotIron_C", 288, 256);
const rods = await group("Build_ConstructorMk1_C", "Recipe_IronRod_C", 608, 96);
const plates = await group("Build_ConstructorMk1_C", "Recipe_IronPlate_C", 608, 416);
const screws = await group("Build_ConstructorMk1_C", "Recipe_Screw_C", 896, 96);
const rip = await group("Build_AssemblerMk1_C", "Recipe_IronPlateReinforced_C", 1184, 416);
const mf = await group("Build_AssemblerMk1_C", "Recipe_ModularFrame_C", 1312, 176);

const G = (id) => ({ kind: "group", id });
const P = (id) => ({ kind: "port", id });
const belt = (from, to, item, tier) => edit([{ type: "add_edge", factory: fid, from, to, item, tier }]);

await belt(P(inP), G(smelt), "Desc_OreIron_C", 3);
await belt(G(smelt), G(rods), "Desc_IronIngot_C", 2);
await belt(G(smelt), G(plates), "Desc_IronIngot_C", 2);
await belt(G(rods), G(screws), "Desc_IronRod_C", 1);
await belt(G(rods), G(mf), "Desc_IronRod_C", 1);
await belt(G(plates), G(rip), "Desc_IronPlate_C", 1);
await belt(G(screws), G(rip), "Desc_IronScrew_C", 1);
await belt(G(rip), G(mf), "Desc_IronPlateReinforced_C", 1);
await belt(G(mf), P(outP), "Desc_ModularFrame_C", 1);

// a second, unclaimed-copper factory pin for map texture
await edit([
  { type: "create_factory", name: "COPPER BASIN", position: { x: -900, y: 1150 }, region: "GRASS FIELDS" },
]);

// target: 2 modular frames/min → screw belt Mk.1 runs at 36/60 = 60%
const r = await edit([{ type: "set_port_rate", id: outP, rate: 2.0 }]);
console.log("seeded. power:", r.derived.factories[fid].totalPowerMw.toFixed(1), "MW");
