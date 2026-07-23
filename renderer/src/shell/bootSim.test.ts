// MANIFOLD boot-sim contract (handoff §4a/§7): growth tracks real progress,
// never resets mid-load; sinks land at their bus-offset thresholds; item
// dots ride only landed legs. Pure math — no DOM, no timers.

import { describe, expect, it } from "vitest";
import { HALF, LEGS, OFFS, legLive, legPoint, minSplashSeconds, newBootSim, stepBootSim } from "./bootSim";

const run = (sim: ReturnType<typeof newBootSim>, seconds: number, target: number, dt = 1 / 60) => {
  for (let t = 0; t < seconds; t += dt) stepBootSim(sim, dt, target);
};

describe("bootSim", () => {
  it("is strictly monotonic even when the reported target rewinds", () => {
    const sim = newBootSim();
    run(sim, 1.0, 0.6);
    const before = sim.prog;
    expect(before).toBeGreaterThan(0.5);
    run(sim, 0.5, 0.1); // a buggy loader reports LESS work done
    expect(sim.prog).toBeGreaterThanOrEqual(before);
  });

  it("stalls hold the bus in place while wall time keeps advancing", () => {
    const sim = newBootSim();
    run(sim, 1.5, 0.4);
    const held = sim.prog;
    const t0 = sim.t;
    run(sim, 2.0, 0.4); // loader stalled
    expect(sim.prog).toBeCloseTo(held, 1);
    expect(sim.t - t0).toBeCloseTo(2.0, 1);
  });

  it("lands the center sink at ~2% and the pairs at their bus offsets", () => {
    const sim = newBootSim();
    run(sim, 0.3, 0.05);
    expect(sim.land[0]).toBeGreaterThanOrEqual(0); // center landed
    expect(sim.land[1]).toBe(-1);
    run(sim, 3.0, 0.5); // smoothed prog approaches 0.5 > 127/381 ≈ 0.333
    expect(sim.land[1]).toBeGreaterThanOrEqual(0);
    expect(sim.land[2]).toBe(-1); // 254/381 ≈ 0.667 not yet crossed
    run(sim, 4.0, 1);
    expect(sim.land[2]).toBeGreaterThanOrEqual(0);
    expect(sim.land[3]).toBeGreaterThanOrEqual(0);
    expect(sim.prog).toBe(1);
    expect(sim.done).toBeGreaterThan(0);
  });

  it("landing timestamps are ordered center-out, matching the bus reach", () => {
    const sim = newBootSim();
    run(sim, 12, 1);
    for (let i = 1; i < OFFS.length; i++) {
      expect(sim.land[i]).toBeGreaterThan(sim.land[i - 1]);
    }
  });

  it("item dots never ride a leg whose sink has not landed", () => {
    const sim = newBootSim();
    run(sim, 1.2, 0.35); // center + first pair landed; outer pairs not
    for (const off of LEGS) {
      const i = OFFS.indexOf(Math.abs(off) as (typeof OFFS)[number]);
      if (sim.land[i] < 0) {
        expect(legLive(sim, off)).toBe(false);
      }
    }
    // the 0.5s tap draw-in gates a just-landed leg too
    const justLanded = OFFS.findIndex((_, i) => sim.land[i] >= 0 && sim.t - sim.land[i] < 0.5);
    if (justLanded >= 0) expect(legLive(sim, OFFS[justLanded])).toBe(false);
  });

  it("minSplashSeconds floors an empty plan and scales up with empire size to a cap", () => {
    // Empty / tiny plan → the base floor (a couple seconds), so even a warm
    // sub-second hydrate plays the full animation.
    expect(minSplashSeconds(0)).toBeCloseTo(2.4, 5);
    expect(minSplashSeconds(0)).toBeGreaterThanOrEqual(2.0);
    // Bigger empires linger longer — strictly monotonic in factory count…
    expect(minSplashSeconds(10)).toBeGreaterThan(minSplashSeconds(0));
    expect(minSplashSeconds(40)).toBeGreaterThan(minSplashSeconds(10));
    // …but clamped so it never drags on forever.
    expect(minSplashSeconds(10_000)).toBe(5.0);
    // A nonsense negative count can't drop below the base floor.
    expect(minSplashSeconds(-5)).toBeCloseTo(2.4, 5);
  });

  it("legPoint traces source → bus → sink, ending at the sink column", () => {
    for (const off of [-HALF, 0, 254]) {
      const [x0, y0] = legPoint(off, 0);
      expect(x0).toBe(640);
      expect(y0).toBeCloseTo(260, 5);
      const [x1] = legPoint(off, 1);
      expect(x1).toBe(640 + off);
    }
  });
});
