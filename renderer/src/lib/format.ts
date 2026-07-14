// Number formatting: every number in the app is JetBrains Mono tabular-nums;
// rates carry an attached, smaller unit; projected values are italic (CSS).

export function fmtRate(v: number): string {
  if (!isFinite(v)) return "∞";
  const abs = Math.abs(v);
  if (abs >= 1000) return v.toFixed(0);
  if (abs >= 100) return v.toFixed(1).replace(/\.0$/, "");
  const s = v.toFixed(2);
  return s.replace(/\.?0+$/, "") || "0";
}

export function fmtPercent(v: number): string {
  return `${Math.round(v * 100)}%`;
}

export function fmtClock(v: number): string {
  const pct = v * 100;
  return `${pct % 1 === 0 ? pct.toFixed(0) : pct.toFixed(1)}%`;
}

/** m:ss clock from seconds — the transport math block's time grammar. Round the
 *  TOTAL first, then split, so 119.7s reads "2:00", never "1:60". Shared by the
 *  route inspector, the train-answer block, and the clipboard payload. */
export function fmtClockS(s: number): string {
  const t = Math.max(0, Math.round(s));
  const m = Math.floor(t / 60);
  const sec = t % 60;
  return `${m}:${String(sec).padStart(2, "0")}`;
}

export function fmtPower(mw: number): string {
  if (mw >= 1000) return `${(mw / 1000).toFixed(1)} GW`;
  return `${mw % 1 === 0 ? mw.toFixed(0) : mw.toFixed(1)} MW`;
}

export function fmtKm(meters: number): string {
  return meters >= 1000 ? `${(meters / 1000).toFixed(1)} km` : `${Math.round(meters)} m`;
}

/** Human duration from minutes: "5h 12m", "16h", "40m". "—" for a
 *  non-finite or non-positive input (e.g. a zero/absent production rate). */
export function fmtDuration(minutes: number): string {
  if (!isFinite(minutes) || minutes <= 0) return "—";
  const total = Math.floor(minutes);
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

/** Flow level from saturation (UI spec thresholds: <70 OK, 70–95 WARN, ≥95 CRIT). */
export type FlowLevel = "ok" | "warn" | "crit";
export function flowLevel(saturation: number): FlowLevel {
  if (saturation >= 0.95) return "crit";
  if (saturation >= 0.7) return "warn";
  return "ok";
}

/** Circuit headroom: fraction of generation left as margin (SDD §12; mirrors
 *  the Rust `circuit_level` formula). Demand with no generation reads fully
 *  overdrawn (-1); an idle grid reads full margin (1). */
export function circuitHeadroom(generationMw: number, demandMw: number): number {
  if (generationMw > 0) return (generationMw - demandMw) / generationMw;
  return demandMw > 0 ? -1 : 1;
}

/** Power level from headroom (thresholds: ≥20% OK, 5–20% WARN, <5% CRIT). The
 *  single source for the audit rows, the PWR chip, and the review banner. */
export function powerLevel(headroom: number): FlowLevel {
  if (headroom < 0.05) return "crit";
  if (headroom < 0.2) return "warn";
  return "ok";
}

/** Honest fallback for item classes the bundled catalog doesn't know:
 *  Desc_SteelIngot_C → "Steel Ingot" (never leak raw class names to chips). */
export function prettyClass(cls: string): string {
  return cls
    .replace(/^Desc_/, "")
    .replace(/_C$/, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2");
}
