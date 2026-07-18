// Eased, continuous wheel zoom for the world map. Leaflet's built-in wheel zoom
// snaps in discrete steps (zoomSnap) — one animated jump per accumulated wheel
// notch, which reads as "chunky". Instead we accumulate a TARGET zoom from wheel
// deltas and ease the live zoom toward it on a requestAnimationFrame loop,
// zooming around the cursor so the point under the mouse stays put.
//
// The whole map (terrain image + nodes + routes) is drawn on a canvas layer —
// there is NO Leaflet tile layer to thrash — so a per-frame instant
// `setZoomAround(..., {animate:false})` redraws crisp each frame and the ease
// itself carries the smoothness. Requires the map created with `zoomSnap: 0` so
// the eased fractional zooms land cleanly, and `scrollWheelZoom: false` so
// Leaflet's own handler doesn't double-zoom.

import L from "leaflet";
import { easeZoomStep } from "./zoomEase";

export interface SmoothZoomOptions {
  /** Per-frame fraction of the remaining gap to close (ease-out strength). */
  approach?: number;
  /** Wheel pixels → zoom units. Trackpads/most mice report pixels (deltaMode 0);
   *  line-mode wheels (deltaMode 1) are normalized at ~16px per line. */
  zoomPerPixel?: number;
  /** Settle threshold in zoom units. */
  epsilon?: number;
}

/** Attach eased wheel zoom to `map`, listening on `container`. Returns a cleanup
 *  that detaches the listener and cancels any in-flight ease. */
export function attachSmoothWheelZoom(
  map: L.Map,
  container: HTMLElement,
  opts: SmoothZoomOptions = {},
): () => void {
  const approach = opts.approach ?? 0.22;
  const zoomPerPixel = opts.zoomPerPixel ?? 0.0018;
  const epsilon = opts.epsilon ?? 0.002;

  let target = map.getZoom();
  let anchor = map.getCenter();
  let raf: number | null = null;

  const clamp = (z: number) => Math.max(map.getMinZoom(), Math.min(map.getMaxZoom(), z));

  const frame = () => {
    const cur = map.getZoom();
    const next = easeZoomStep(cur, target, approach, epsilon);
    map.setZoomAround(anchor, next, { animate: false });
    raf = next === target ? null : requestAnimationFrame(frame);
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const rect = container.getBoundingClientRect();
    anchor = map.containerPointToLatLng(L.point(e.clientX - rect.left, e.clientY - rect.top));
    // A fresh gesture (no ease in flight) resyncs the target to the live zoom,
    // so a preceding button/programmatic zoom can't leave the target stale.
    if (raf === null) target = map.getZoom();
    const px = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    target = clamp(target - px * zoomPerPixel);
    if (raf === null) raf = requestAnimationFrame(frame);
  };

  container.addEventListener("wheel", onWheel, { passive: false });
  return () => {
    container.removeEventListener("wheel", onWheel);
    if (raf !== null) cancelAnimationFrame(raf);
  };
}
