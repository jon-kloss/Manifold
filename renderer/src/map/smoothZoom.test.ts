import { describe, it, expect } from "vitest";
import { easeZoomStep } from "./zoomEase";

describe("easeZoomStep", () => {
  it("moves a fraction of the remaining gap toward the target (ease-out)", () => {
    // from 2 toward 4, closing 25% of the gap → 2 + 0.25*2 = 2.5
    expect(easeZoomStep(2, 4, 0.25, 0.002)).toBeCloseTo(2.5, 6);
  });

  it("snaps exactly to the target once within epsilon (no asymptote)", () => {
    expect(easeZoomStep(3.999, 4, 0.25, 0.01)).toBe(4);
  });

  it("eases downward symmetrically", () => {
    expect(easeZoomStep(4, 2, 0.5, 0.002)).toBeCloseTo(3, 6);
  });

  it("converges monotonically to the target within a bounded number of frames", () => {
    let z = 1;
    const target = 6;
    let frames = 0;
    while (z !== target && frames < 200) {
      const next = easeZoomStep(z, target, 0.22, 0.002);
      expect(next).toBeGreaterThanOrEqual(z); // never overshoots upward
      expect(next).toBeLessThanOrEqual(target);
      z = next;
      frames++;
    }
    expect(z).toBe(target);
    expect(frames).toBeLessThan(100); // settles quickly (~tens of frames)
  });
});
