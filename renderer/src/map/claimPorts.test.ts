import { describe, it, expect } from "vitest";
import { orphanClaimPorts } from "./claimPorts";

// #120: ports left behind by released claims must be found (for reuse) while
// ports backed by live claims stay untouchable.
describe("orphanClaimPorts", () => {
  const p = (id: string, rateCeiling: number | null) => ({ id, rateCeiling });

  it("no claims → every claim-shaped port is an orphan", () => {
    expect(orphanClaimPorts([p("a", 60), p("b", 120)], [])).toEqual([p("a", 60), p("b", 120)]);
  });

  it("each live claim consumes exactly one rate-matched port", () => {
    expect(orphanClaimPorts([p("a", 60), p("b", 60), p("c", 120)], [60, 120])).toEqual([p("b", 60)]);
  });

  it("fully-backed ports → no orphans", () => {
    expect(orphanClaimPorts([p("a", 60), p("b", 120)], [120, 60])).toEqual([]);
  });

  it("a claim with no matching port consumes nothing (tier drift on the claim side)", () => {
    // claim says 120 but only a 60 port exists — the 60 port is NOT an orphan
    // for certain, but greedy matching can't pair them; err on the side of
    // reporting it (reuse tunes the ceiling anyway, add would duplicate).
    expect(orphanClaimPorts([p("a", 60)], [120])).toEqual([p("a", 60)]);
  });

  it("null-ceiling ports never match a claim rate", () => {
    expect(orphanClaimPorts([p("open", null), p("a", 60)], [60])).toEqual([p("open", null)]);
  });

  it("ceilings within ±0.5 count as matching (float drift)", () => {
    expect(orphanClaimPorts([p("a", 59.8)], [60])).toEqual([]);
  });
});
