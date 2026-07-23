// MANIFOLD boot choreography state (handoff §4a/§7) — the pure math behind
// BootScreen, kept DOM-free so the contract is unit-testable:
//   • the growing network IS the progress bar: the bus reach follows the REAL
//     load fraction through a lerp smoother and is STRICTLY MONOTONIC — it
//     never rewinds mid-load, even if a caller misreports a lower fraction;
//   • sink pairs land exactly when smoothed progress crosses their bus
//     offsets (2% for the center sink, then 33% / 67% / 100% pairs), each
//     recording its landing timestamp for the pop/bloom/ring animation;
//   • item dots ride ONLY legs whose sink has landed (and only after its tap
//     has had 0.5s to draw), on wall time — so stalls stay alive without a
//     single synthetic progress tick.

/** Geometry (1280x720 design grid, from the handoff prototype). */
export const CX = 640;
export const SRC_Y = 240;
export const BUS_Y = 330;
export const TAP_Y = 430;
export const SINK_Y = 446;
export const HALF = 381;
/** Sink bus offsets: center + 3 symmetric pairs. */
export const OFFS = [0, 127, 254, 381] as const;
/** All seven legs, left to right (negative = left of center). */
export const LEGS = [-381, -254, -127, 0, 127, 254, 381] as const;

export interface BootSim {
  /** wall-clock seconds since the sim started */
  t: number;
  /** smoothed, strictly monotonic progress 0..1 */
  prog: number;
  /** seconds spent at prog == 1 (drives the done-beat) */
  done: number;
  /** landing timestamp per OFFS entry, -1 = not landed */
  land: number[];
}

export const newBootSim = (): BootSim => ({ t: 0, prog: 0, done: 0, land: [-1, -1, -1, -1] });

/**
 * Advance the sim by dt seconds toward the loader-reported target fraction.
 * The smoother `prog += (target − prog) · min(1, dt·4)` eases stalls and
 * bursts; once the target reports complete, a small constant drive closes the
 * asymptotic gap so the bus visibly finishes.
 */
export function stepBootSim(sim: BootSim, dt: number, target: number): void {
  sim.t += dt;
  const clamped = Math.max(0, Math.min(1, target));
  const next = sim.prog + (clamped - sim.prog) * Math.min(1, dt * 4) + (clamped >= 1 ? dt * 0.25 : 0);
  // Strictly monotonic: a rewinding target (a loader bug) must never retract
  // the built network mid-load.
  sim.prog = Math.min(1, Math.max(sim.prog, next));
  for (let i = 0; i < OFFS.length; i++) {
    const threshold = OFFS[i] === 0 ? 0.02 : OFFS[i] / HALF;
    if (sim.land[i] < 0 && sim.prog >= threshold - 0.002) sim.land[i] = sim.t;
  }
  if (sim.prog >= 0.999) {
    sim.prog = 1;
    sim.done += dt;
  }
}

/** May item dots ride the leg at bus offset `off` yet? Only once its sink has
 *  landed and the tap line has had its 0.5s draw-in. */
export function legLive(sim: BootSim, off: number): boolean {
  const i = OFFS.indexOf(Math.abs(off) as (typeof OFFS)[number]);
  return i >= 0 && sim.land[i] >= 0 && sim.t - sim.land[i] >= 0.5;
}

/** Point along a leg's source → bus → sink path at parameter u ∈ [0,1]. */
export function legPoint(off: number, u: number): [number, number] {
  const l1 = BUS_Y - SRC_Y - 20;
  const l2 = Math.abs(off);
  const l3 = SINK_Y - 18 - BUS_Y;
  const total = l1 + l2 + l3;
  let d = u * total;
  if (d <= l1) return [CX, SRC_Y + 20 + d];
  d -= l1;
  if (d <= l2) return [CX + Math.sign(off) * d, BUS_Y];
  d -= l2;
  return [CX + off, BUS_Y + d];
}

/**
 * Minimum wall seconds the animated splash holds before revealing the map — a
 * floor so the boot choreography always plays in full instead of a warm,
 * sub-second hydrate flashing past. Scales UP with empire size (a bigger plan is
 * more to take in) at `perFactory` per factory, clamped to `[min, max]`. A
 * genuinely slow load already outlives this floor, so it's a no-op there.
 */
export const minSplashSeconds = (
  factoryCount: number,
  min = 2.4,
  perFactory = 0.06,
  max = 5.0,
): number => Math.min(max, min + Math.max(0, factoryCount) * perFactory);

export const c01 = (v: number) => Math.max(0, Math.min(1, v));
export const seg = (p: number, a: number, b: number) => c01((p - a) / (b - a));
/** ease-out cubic */
export const eoc = (t: number) => 1 - Math.pow(1 - t, 3);
/** back-ease overshoot (the sink landing pop) */
export const back = (t: number) => {
  const c = 2.0;
  const u = t - 1;
  return 1 + (c + 1) * u * u * u + c * u * u;
};
