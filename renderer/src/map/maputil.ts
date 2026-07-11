// World meters ↔ Leaflet CRS.Simple coordinates. 1 map unit = 50 m; north up.

import L from "leaflet";
import type { GameMachine, MapPos } from "../state/types";

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
  const purityFactor = purity === "pure" ? 2 : purity === "impure" ? 0.5 : 1;
  return base * purityFactor * clock;
}

export const EXTRACTORS = ["Build_MinerMk1_C", "Build_MinerMk2_C", "Build_MinerMk3_C"];
