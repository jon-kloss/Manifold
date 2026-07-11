// Top-down machine footprints in meters (width × length), community-documented
// build dimensions. Rendered at one shared scale so relative size reads
// truthfully across machines. Docs.json carries no dimensions — this table is
// the presentation-side source until pak extraction lands (v2, SDD §7).

export interface Footprint {
  w: number;
  l: number;
}

const FOOTPRINTS: Record<string, Footprint> = {
  Build_SmelterMk1_C: { w: 6, l: 9 },
  Build_ConstructorMk1_C: { w: 8, l: 10 },
  Build_AssemblerMk1_C: { w: 10, l: 15 },
  Build_FoundryMk1_C: { w: 10, l: 9 },
  Build_ManufacturerMk1_C: { w: 18, l: 20 },
  Build_OilRefinery_C: { w: 10, l: 20 },
  Build_Packager_C: { w: 8, l: 8 },
  Build_Blender_C: { w: 18, l: 16 },
  Build_HadronCollider_C: { w: 24, l: 38 },
  Build_ConveyorAttachmentSplitter_C: { w: 4, l: 4 },
  Build_ConveyorAttachmentSplitterSmart_C: { w: 4, l: 4 },
  Build_ConveyorAttachmentSplitterProgrammable_C: { w: 4, l: 4 },
  Build_ConveyorAttachmentMerger_C: { w: 4, l: 4 },
  Build_StorageContainerMk1_C: { w: 5, l: 10 },
  Build_StorageContainerMk2_C: { w: 5, l: 10 },
  Build_MinerMk1_C: { w: 6, l: 14 },
  Build_MinerMk2_C: { w: 6, l: 14 },
  Build_MinerMk3_C: { w: 6, l: 14 },
};

const FALLBACK: Footprint = { w: 8, l: 8 };

export function footprintOf(machineClass: string): Footprint {
  return FOOTPRINTS[machineClass] ?? FALLBACK;
}

/** Shared render scale: px per meter in the card footprint strip. */
export const FOOTPRINT_SCALE = 1.1;

export function footprintArea(machineClass: string, count: number): number {
  const f = footprintOf(machineClass);
  return f.w * f.l * count;
}
