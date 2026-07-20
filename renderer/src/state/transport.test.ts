import { describe, it, expect } from "vitest";
import {
  clampEdgeTier,
  isFluidItem,
  transportCapacity,
  transportTiers,
  type GameData,
} from "./types";

// A minimal catalog: one fluid, one solid.
const gd = {
  items: {
    Desc_Water_C: { className: "Desc_Water_C", displayName: "Water", form: "RF_LIQUID", stackSize: "0" },
    Desc_IronIngot_C: { className: "Desc_IronIngot_C", displayName: "Iron Ingot", form: "RF_SOLID", stackSize: "100" },
  },
} as unknown as GameData;

describe("clampEdgeTier", () => {
  it("clamps a stale belt tier on a fluid edge into the pipe range (1..2)", () => {
    // A plan saved before pipes existed can carry a water edge at belt tier 6.
    // The tier <select> only offers [1,2] for fluids, so the value must clamp
    // to a real option (else React renders a blank select).
    expect(clampEdgeTier(gd, "Desc_Water_C", 6)).toBe(2);
    expect(clampEdgeTier(gd, "Desc_Water_C", 3)).toBe(2);
    expect(clampEdgeTier(gd, "Desc_Water_C", 2)).toBe(2);
    expect(clampEdgeTier(gd, "Desc_Water_C", 1)).toBe(1);
    expect(clampEdgeTier(gd, "Desc_Water_C", 0)).toBe(1);
    // The clamped value is always one of the offered options.
    const opts = transportTiers(gd, "Desc_Water_C");
    expect(opts).toContain(clampEdgeTier(gd, "Desc_Water_C", 6));
  });

  it("leaves solid (belt) tiers untouched across the full Mk.1–6 range", () => {
    for (const t of [1, 3, 6]) expect(clampEdgeTier(gd, "Desc_IronIngot_C", t)).toBe(t);
    expect(clampEdgeTier(gd, "Desc_IronIngot_C", 9)).toBe(6);
  });

  it("capacity reads the clamped medium: a tier-6 water edge is a Mk.2 pipe (600), not a belt", () => {
    expect(isFluidItem(gd, "Desc_Water_C")).toBe(true);
    expect(transportCapacity(gd, "Desc_Water_C", 6)).toBe(600); // pipe Mk.2, not belt Mk.6 (1200)
    expect(transportCapacity(gd, "Desc_IronIngot_C", 6)).toBe(1200); // belt Mk.6
  });
});
