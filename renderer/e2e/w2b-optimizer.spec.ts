// W2b-D empire alternate-recipe optimizer: the ALT OPTIMIZER audit tab renders
// through the real Rust core. HONEST fixture coverage — the trimmed fixture
// catalog ships no unlocked alternates, so the read-only ranking is empty; the
// e2e asserts the tab mounts, shows its empty state, and does NOT break the
// audit drawer or its other tabs. The ranking + CTA routing (◇→T2, ◆→Refactor,
// ◆ never mutated) are proven deterministically by the Rust altopt tests, which
// can inject synthetic unlocked alternates the shared serial e2e plan cannot.

import { test, expect } from "@playwright/test";

test("ALT OPTIMIZER tab renders (empty in fixture) without breaking the drawer", async ({ page }) => {
  await page.goto("/");
  await page.keyboard.press("Escape"); // dismiss any auto-presented dashboard
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });

  // Open the audit drawer (TAB toggles the HUD).
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();

  // The read-only endpoint returns [] when nothing is unlocked (honest).
  const opps = await (await page.request.get("/api/optimize/empire")).json();
  expect(Array.isArray(opps)).toBe(true);
  expect(opps.length).toBe(0);

  // Switch to the ALT OPTIMIZER tab → its empty state renders, no rows.
  await page.locator(".audit-tab", { hasText: "ALT OPTIMIZER" }).click();
  await expect(page.getByTestId("audit-drawer")).toContainText("No unlocked alternates to weigh");
  await expect(page.getByTestId("optimizer-row")).toHaveCount(0);

  // The drawer and its other tabs still work — the new tab is additive.
  await page.locator(".audit-tab", { hasText: "SATURATION" }).click();
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
});
