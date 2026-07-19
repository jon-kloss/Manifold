// Audit #125 acceptance (promoted from the audit probe suite, restructured):
// multi-output deficit gating and sizing must follow the EDITED output's
// requested rate — never the factory's FIRST Out port. Before the fix, the
// clamped-channel deficit (sized on the SetTarget edit response) took
// `requested` from the first Out port, so an unrelated sibling target both
// fired PHANTOM deficits and mis-scaled `needed`.
//
// NOTE ON THE DRIVE: a clamped SetTarget writes the achieved rate back to the
// canonical port, so a later /api/hydrate sees a MET target and reports no
// deficit — the clamped channel is only observable on the edit RESPONSE
// itself. The original audit probe read hydrate (the degraded channel, which
// was never wrong); this promoted version drives the fixed path directly.
//
// Fixture catalog (dev bridge default):
//   Recipe_IronRod_C = 1 iron ingot -> 1 rod   @ 4s => 15/min per machine
//   Recipe_Wire_C    = 1 copper     -> 2 wire  @ 4s => 30 wire/min per machine
//   Recipe_IngotCopper_C = 1 ore -> 1 ingot @ 2s => 30/min per machine
//   Belt Mk.1 caps a route at 60/min.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
// Full edit response — the clamped-channel deficit rides on the response's
// derived, not on a later hydrate (see NOTE ON THE DRIVE above).
async function editFull(request: APIRequestContext, cmds: unknown[]): Promise<any> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

const P = (id: string) => ({ kind: "port", id });
const G = (id: string) => ({ kind: "group", id });
const mk = async (request: APIRequestContext, name: string, x: number, y: number) =>
  (await edit(request, [{ type: "create_factory", name, position: { x, y }, region: "GRASS FIELDS" }])).created[0];
const port = async (
  request: APIRequestContext,
  factory: string,
  direction: string,
  item: string,
  ceiling: number | null,
  x: number,
  y = 100,
) =>
  (await edit(request, [{ type: "add_port", factory, direction, item, rate: 0, rateCeiling: ceiling, graphPos: { x, y } }]))
    .created[0];
const group = async (
  request: APIRequestContext,
  factory: string,
  machine: string,
  recipe: string,
  count = 1,
  x = 300,
  y = 100,
) => (await edit(request, [{ type: "add_group", factory, machine, recipe, count, clock: 1.0, graphPos: { x, y }, floor: 0 }])).created[0];
const belt = (request: APIRequestContext, factory: string, from: unknown, to: unknown, item: string, tier = 6) =>
  edit(request, [{ type: "add_edge", factory, from, to, item, tier }]);

const deficitFor = (resp: any, portId: string) =>
  (resp.derived.deficits as any[]).find((d) => d.port === portId);

// ---------------------------------------------------------------------------
// PROBE — Multi-output clamped-channel deficits follow the edited output.
//
// Factory F, two INDEPENDENT chains:
//   chain A (created FIRST → its Out is the factory's first Out port):
//     open iron-ingot In → rod constructors → outA (rod)
//   chain B: route-capped copper In (60/min via Mk.1 belt) → wire
//     constructors → outB (wire)
//
// EXPECTED:
//   (1) With A=100, editing B→400 (needs 200 copper, only 60 supplied) sizes
//       the deficit at B's TRUE requirement: needed == 200 copper/min.
//   (2) Raising only A to 200 and re-editing B→400 leaves needed unchanged
//       (200): B's requirement is independent of A's target. (Old code scaled
//       by A's rate: 50 then 100 — both wrong and A-coupled.)
//   (3) Editing B→100 (needs 50 ≤ 60 supplied — MET) reports NO deficit for
//       B's input, even though A's 200 target exceeds B's 120/min ceiling
//       (the old gate compared A.rate against B's max_rate → phantom row).
// ---------------------------------------------------------------------------
test("multi-output deficit follows the edited output, not the first out port", async ({ request }) => {
  await resetView(request);
  const f = await mk(request, "SESSION MULTI-OUT", -2600, 2600);
  const src = await mk(request, "SESSION COPPER SRC", -3400, 2600);
  try {
    // ---- F chain A (FIRST): open ingot input -> rods ----
    const inIron = await port(request, f, "in", "Desc_IronIngot_C", null, 0, 60);
    const outA = await port(request, f, "out", "Desc_IronRod_C", null, 640, 60); // first Out port
    const rod = await group(request, f, "Build_ConstructorMk1_C", "Recipe_IronRod_C", 20, 320, 60);
    await belt(request, f, P(inIron), G(rod), "Desc_IronIngot_C");
    await belt(request, f, G(rod), P(outA), "Desc_IronRod_C");
    // ---- F chain B: copper In + wire Out ----
    const inY = await port(request, f, "in", "Desc_CopperIngot_C", null, 0, 220);
    const outB = await port(request, f, "out", "Desc_Wire_C", null, 640, 220);
    const wire = await group(request, f, "Build_ConstructorMk1_C", "Recipe_Wire_C", 20, 320, 220);
    await belt(request, f, P(inY), G(wire), "Desc_CopperIngot_C");
    await belt(request, f, G(wire), P(outB), "Desc_Wire_C");

    // ---- SRC: ore -> copper ingot, shipped 120/min but Mk.1-route-capped to 60 ----
    const oreIn = await port(request, src, "in", "Desc_OreCopper_C", null, 0);
    const copperOut = await port(request, src, "out", "Desc_CopperIngot_C", null, 640);
    const smelt = await group(request, src, "Build_SmelterMk1_C", "Recipe_IngotCopper_C", 10);
    await belt(request, src, P(oreIn), G(smelt), "Desc_OreCopper_C");
    await belt(request, src, G(smelt), P(copperOut), "Desc_CopperIngot_C");
    await edit(request, [{ type: "set_port_rate", id: copperOut, rate: 120 }]);
    await edit(request, [
      {
        type: "add_route",
        kind: { kind: "belt", tier: 1 },
        from: copperOut,
        to: inY,
        path: [{ x: -3400, y: 2600 }, { x: -2600, y: 2600 }],
      },
    ]);

    // (1) A=100; edit B -> 400: needed == B's true requirement (200 copper).
    await edit(request, [{ type: "set_port_rate", id: outA, rate: 100 }]);
    const r1 = await editFull(request, [{ type: "set_port_rate", id: outB, rate: 400 }]);
    const d1 = deficitFor(r1, inY);
    expect(d1, "starved wire chain reports a deficit on its copper input").toBeTruthy();
    expect(d1.needed).toBeCloseTo(200, 3);
    expect(d1.supplied).toBeCloseTo(60, 3);

    // (2) bump ONLY A to 200; re-edit B -> 400 (state was clamp-written to
    //     120, so this is a real edit): needed is unmoved by A's target.
    await edit(request, [{ type: "set_port_rate", id: outA, rate: 200 }]);
    const r2 = await editFull(request, [{ type: "set_port_rate", id: outB, rate: 400 }]);
    const d2 = deficitFor(r2, inY);
    expect(d2, "still starved after A's target changes").toBeTruthy();
    expect(Math.abs(d2.needed - d1.needed)).toBeLessThanOrEqual(1e-6);

    // (3) phantom gate: B -> 100 is fully covered (50 of 60 copper) — no
    //     deficit row for inY, despite A's 200 exceeding B's 120 ceiling.
    const r3 = await editFull(request, [{ type: "set_port_rate", id: outB, rate: 100 }]);
    expect(deficitFor(r3, inY), "met target must not report a phantom deficit").toBeFalsy();
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: src }]).catch(() => {});
  }
});
