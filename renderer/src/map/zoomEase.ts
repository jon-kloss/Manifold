// Pure easing math for the world map's smooth wheel zoom, kept free of any
// Leaflet import so it is unit-testable in the node test env (leaflet needs a
// DOM at import). smoothZoom.ts is the Leaflet glue that drives this per frame.

/** Exponential ease-out toward `target`: each frame closes `approach` of the
 *  remaining gap. Returns `target` exactly once within `epsilon` so the rAF
 *  loop settles (no infinite asymptote). */
export function easeZoomStep(cur: number, target: number, approach: number, epsilon: number): number {
  return Math.abs(target - cur) < epsilon ? target : cur + (target - cur) * approach;
}
