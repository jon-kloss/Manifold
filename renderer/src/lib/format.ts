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

export function fmtPower(mw: number): string {
  if (mw >= 1000) return `${(mw / 1000).toFixed(1)} GW`;
  return `${mw % 1 === 0 ? mw.toFixed(0) : mw.toFixed(1)} MW`;
}

export function fmtKm(meters: number): string {
  return meters >= 1000 ? `${(meters / 1000).toFixed(1)} km` : `${Math.round(meters)} m`;
}

/** Flow level from saturation (UI spec thresholds: <70 OK, 70–95 WARN, ≥95 CRIT). */
export type FlowLevel = "ok" | "warn" | "crit";
export function flowLevel(saturation: number): FlowLevel {
  if (saturation >= 0.95) return "crit";
  if (saturation >= 0.7) return "warn";
  return "ok";
}
