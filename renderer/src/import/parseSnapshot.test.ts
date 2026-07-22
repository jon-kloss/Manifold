import { describe, it, expect } from "vitest";
import { buildSnapshot, type RawObject } from "./parseSnapshot";

// Minimal synthetic object-graph builder. Only the fields the reducer reads.
function obj(
  typePath: string,
  props?: Record<string, unknown>,
  pos: { x: number; y: number; z: number } = { x: 100, y: 200, z: 0 },
): RawObject {
  return { typePath, transform: { translation: pos }, properties: props };
}

function recipe(pathName: string) {
  return { mCurrentRecipe: { value: { pathName } } };
}

function snap(objects: RawObject[]) {
  return buildSnapshot("Test", "495413", { Persistent_Level: { objects } });
}

describe("buildSnapshot — DC-H1 unrecognized-vanilla-producer backstop", () => {
  it("(a) surfaces an unrecognized VANILLA producer (Build_* in /Game/FactoryGame with a recipe) into quarantined", () => {
    const s = snap([
      obj(
        "/Game/FactoryGame/Buildable/Factory/Foo/Build_Foo_C.Build_Foo_C",
        recipe("/Game/FactoryGame/Recipes/Recipe_Foo_C.Recipe_Foo_C"),
      ),
    ]);
    // Not a recognized machine — must not be counted as production…
    expect(s.machines).toHaveLength(0);
    // …but must NOT be silently dropped: it appears as a breadcrumb.
    expect(s.quarantined?.["Build_Foo_C"]).toBe(1);
  });

  it("(b) a recognized manufacturer with a recipe becomes a machine and is NOT double-counted in quarantined", () => {
    const s = snap([
      obj(
        "/Game/FactoryGame/Buildable/Factory/ConstructorMk1/Build_ConstructorMk1_C.Build_ConstructorMk1_C",
        recipe("/Game/FactoryGame/Recipes/Recipe_IronRod_C.Recipe_IronRod_C"),
      ),
    ]);
    expect(s.machines).toHaveLength(1);
    expect(s.machines[0]).toMatchObject({ class: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C" });
    expect(s.quarantined ?? {}).toEqual({});
  });

  it("(c) Build_GeneratorIntegratedBiomass_C — a catalog-absent world burner with NO recipe — stays recognized-and-ignored (TA-M4: never a generator, never surfaced)", () => {
    const s = snap([
      // No mCurrentRecipe: it is not a producer. Appears twice in every save.
      obj(
        "/Game/FactoryGame/Buildable/Factory/GeneratorIntegratedBiomass/Build_GeneratorIntegratedBiomass_C.Build_GeneratorIntegratedBiomass_C",
      ),
      obj(
        "/Game/FactoryGame/Buildable/Factory/GeneratorIntegratedBiomass/Build_GeneratorIntegratedBiomass_C.Build_GeneratorIntegratedBiomass_C",
      ),
    ]);
    expect(s.machines).toHaveLength(0);
    expect(s.quarantined ?? {}).toEqual({});
  });

  it("(c') vanilla decor / world markers / transport stations carry no recipe → recognized-and-ignored, never surfaced", () => {
    const s = snap([
      obj("/Game/FactoryGame/Buildable/Building/Foundation/Build_Foundation_8x1_01_C.Build_Foundation_8x1_01_C"),
      obj("/Game/FactoryGame/Buildable/…/BP_FrackingSatellite_C.BP_FrackingSatellite_C"),
      obj("/Game/FactoryGame/Buildable/Factory/DroneStation/Build_DroneStation_C.Build_DroneStation_C"),
    ]);
    expect(s.machines).toHaveLength(0);
    expect(s.quarantined ?? {}).toEqual({});
  });

  it("(d) modded content (Build_/BP_ outside /Game/FactoryGame/) is still quarantined exactly as before", () => {
    const s = snap([
      obj("/Game/FactoryGame_Mod/Build_ModMachine_C.Build_ModMachine_C", recipe("/x/Recipe_Mod_C.Recipe_Mod_C")),
      obj("/Some/Mod/Path/BP_ModThing_C.BP_ModThing_C"),
    ]);
    expect(s.machines).toHaveLength(0);
    expect(s.quarantined?.["Build_ModMachine_C"]).toBe(1);
    expect(s.quarantined?.["BP_ModThing_C"]).toBe(1);
  });

  it("still recognizes the core model (extractors, belts, trains) alongside the backstop", () => {
    const s = snap([
      obj(
        "/Game/FactoryGame/…/Build_MinerMk1_C.Build_MinerMk1_C",
        { mExtractableResource: { value: { pathName: "Persistent_Level:PersistentLevel.BP_ResourceNode1" } } },
      ),
      obj("/Game/FactoryGame/…/Build_ConveyorBeltMk2_C.Build_ConveyorBeltMk2_C"),
      obj("/Game/FactoryGame/…/Build_TrainStation_C.Build_TrainStation_C"),
    ]);
    expect(s.extractors).toHaveLength(1);
    expect(s.extractors?.[0].nodeActorId).toBe("Persistent_Level:PersistentLevel.BP_ResourceNode1");
    expect(s.belts?.["Build_ConveyorBeltMk2_C"]).toBe(1);
    expect(s.trainStations).toBe(1);
    expect(s.quarantined ?? {}).toEqual({});
  });
});

describe("buildSnapshot — imported generators carry their loaded fuel", () => {
  const coalGen = (fuelPath?: string) =>
    obj(
      "/Game/FactoryGame/Buildable/Factory/GeneratorCoal/Build_GeneratorCoal_C.Build_GeneratorCoal_C",
      fuelPath ? { mCurrentFuelClass: { value: { pathName: fuelPath } } } : undefined,
    );

  it("reads mCurrentFuelClass so Rust can infer the burn recipe", () => {
    const s = snap([coalGen("/Game/FactoryGame/Resource/RawResources/Coal/Desc_Coal_C.Desc_Coal_C")]);
    expect(s.machines).toHaveLength(1);
    // a generator carries fuel, never a recipe (mCurrentRecipe is absent)
    expect(s.machines[0]).toMatchObject({ class: "Build_GeneratorCoal_C", fuel: "Desc_Coal_C", recipe: null });
  });

  it("an idle generator with no fuel loaded reports fuel: null (→ stays nameplate)", () => {
    expect(snap([coalGen()]).machines[0].fuel).toBeNull();
    // ...and the same for the malformed shapes a real save's empty fuel slot
    // serializes as — the `!path` guard's actual job (not just prop-absent).
    const emptyPath = obj(
      "/Game/FactoryGame/Buildable/Factory/GeneratorCoal/Build_GeneratorCoal_C.Build_GeneratorCoal_C",
      { mCurrentFuelClass: { value: { pathName: "" } } },
    );
    expect(snap([emptyPath]).machines[0].fuel).toBeNull();
    const noPathName = obj(
      "/Game/FactoryGame/Buildable/Factory/GeneratorCoal/Build_GeneratorCoal_C.Build_GeneratorCoal_C",
      { mCurrentFuelClass: { value: {} } },
    );
    expect(snap([noPathName]).machines[0].fuel).toBeNull();
  });

  it("a manufacturer never reads a fuel class — even if one is present (the GENERATORS gate)", () => {
    // The constructor carries a stray fuel-class ref AND a recipe. `fuel` must
    // still be null: only generators read fuel. Without the GENERATORS.has(cls)
    // gate this would leak a spurious fuel — so the fuel prop must be present to
    // actually exercise the gate (not pass by coincidental absence).
    const s = snap([
      obj(
        "/Game/FactoryGame/Buildable/Factory/ConstructorMk1/Build_ConstructorMk1_C.Build_ConstructorMk1_C",
        {
          ...recipe("/Game/FactoryGame/Recipes/Recipe_IronRod_C.Recipe_IronRod_C"),
          mCurrentFuelClass: { value: { pathName: "/Game/…/Desc_Coal_C.Desc_Coal_C" } },
        },
      ),
    ]);
    expect(s.machines[0]).toMatchObject({ class: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", fuel: null });
  });
});
