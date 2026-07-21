// World meters ↔ Leaflet CRS.Simple coordinates. 1 map unit = 50 m; north up.

import L from "leaflet";
import { purityFactor, type GameMachine, type MapPos } from "../state/types";

export const METERS_PER_UNIT = 50;

export function toLatLng(pos: MapPos): L.LatLngExpression {
  return [-pos.y / METERS_PER_UNIT, pos.x / METERS_PER_UNIT];
}

export function fromLatLng(ll: L.LatLng): MapPos {
  return { x: ll.lng * METERS_PER_UNIT, y: -ll.lat * METERS_PER_UNIT };
}

/** Extraction ceiling in items/min (twin of gamedata::extraction_rate). */
export function extractionRate(machine: GameMachine | undefined, purity: string, clock: number): number {
  const m = machine as (GameMachine & { itemsPerCycle?: number; cycleTimeS?: number }) | undefined;
  if (!m || m.kind !== "extractor" || !m.itemsPerCycle || !m.cycleTimeS) return 0;
  const base = (m.itemsPerCycle / m.cycleTimeS) * 60;
  return base * purityFactor(purity) * clock;
}

export const EXTRACTORS = ["Build_MinerMk1_C", "Build_MinerMk2_C", "Build_MinerMk3_C"];
/** Fluid nodes take dedicated extractors, never miners: crude oil is pumped
 *  by the Oil Extractor (no Mk tiers, purity-scaled like everything else). */
export const FLUID_EXTRACTORS: Record<string, string[]> = {
  Desc_LiquidOil_C: ["Build_OilPump_C"],
};

/** The extractor classes a node of `item` can legally claim with — miners
 *  for solid ores, the dedicated pump for fluids. Single authority for both
 *  drawer pickers (twin of app::wizard::extractor_for on the MAKE path). */
export function extractorsFor(item: string): string[] {
  return FLUID_EXTRACTORS[item] ?? EXTRACTORS;
}
