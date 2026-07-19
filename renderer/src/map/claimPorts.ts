// #120 — claim ⇄ port lifecycle helpers (leaflet-free: unit-testable in node).
/** #120 — claim-shaped in-ports of one item that no LIVE claim accounts for:
 *  greedily match each claim's extraction rate to a port ceiling (±0.5); the
 *  leftovers are orphans (their claim was released while the port stayed —
 *  deliberately, when wired, so belts survive). Re-claiming reuses an orphan
 *  instead of stacking a duplicate port. */
export function orphanClaimPorts<P extends { id: string; rateCeiling: number | null }>(
  ports: P[],
  liveClaimRates: number[],
): P[] {
  const pool = [...ports];
  for (const rate of liveClaimRates) {
    const i = pool.findIndex((p) => p.rateCeiling != null && Math.abs(p.rateCeiling - rate) < 0.5);
    if (i >= 0) pool.splice(i, 1);
  }
  return pool;
}
