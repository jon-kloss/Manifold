// Smooth eased wheel zoom: one wheel tick must GLIDE the zoom through several
// intermediate values over the ease (not one discrete jump) and land on a
// fractional level (zoomSnap 0) — proving the custom eased handler drives the
// zoom, not Leaflet's stepped wheel zoom. The map stamps its live zoom on
// [data-testid=map-root] data-zoom on every zoom frame (a direct DOM write).

import { test, expect } from "@playwright/test";
import { resetView } from "./helpers";

test("wheel zoom eases through intermediate levels and lands fractional", async ({ page, request }) => {
  await resetView(request);
  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
  const root = page.getByTestId("map-root");
  await expect(root).toBeVisible();
  await page.waitForTimeout(300);
  const box = (await page.locator(".leaflet-container").boundingBox())!;

  // Dispatch one wheel tick at the map center, then sample the live zoom stamp
  // across the ease (rAF for ~450ms), collecting distinct values.
  const { before, after, distinct } = await page.evaluate(
    ({ x, y }) =>
      new Promise<{ before: number; after: number; distinct: number }>((resolve) => {
        const rootEl = document.querySelector('[data-testid="map-root"]') as HTMLElement;
        const el = document.querySelector(".leaflet-container") as HTMLElement;
        const z = () => Number(rootEl.dataset.zoom);
        const before = z();
        const seen = new Set<string>();
        el.dispatchEvent(new WheelEvent("wheel", { deltaY: -240, clientX: x, clientY: y, bubbles: true, cancelable: true }));
        const t0 = performance.now();
        const tick = () => {
          seen.add(rootEl.dataset.zoom ?? "");
          if (performance.now() - t0 < 450) requestAnimationFrame(tick);
          else resolve({ before, after: z(), distinct: seen.size });
        };
        requestAnimationFrame(tick);
      }),
    { x: box.x + box.width / 2, y: box.y + box.height / 2 },
  );

  // Zoom increased...
  expect(after).toBeGreaterThan(before);
  // ...through MULTIPLE distinct intermediate values (a glide, not one jump)...
  expect(distinct).toBeGreaterThanOrEqual(3);
  // ...and landed on a fractional level (not a Leaflet 0.5 snap step).
  expect(after % 0.5).not.toBe(0);
});
