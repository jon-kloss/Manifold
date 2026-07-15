// Hardening: a backend-refused command must surface as the status-bar error
// chip and must NOT desync the UI. Named phase6 so it sorts after the import
// specs — it leans on the ◆ built layer the phase-4 Dunarr import left behind
// (built entities are immutable, so Delete on one is a guaranteed refusal).

import { test, expect } from "@playwright/test";

import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

// Deterministic map boot — never inherit a dead predecessor's viewState.
test.beforeEach(async ({ request }) => resetView(request));

test("a refused delete surfaces in the status bar and leaves the UI intact", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // open a ◆ built factory imported from the save (search → summary → open)
  await page.locator(".searchbox input").fill("iron ingot works 1");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();

  // select a built machine group
  const cards = page.locator(".group-card");
  const cardCount = await cards.count();
  expect(cardCount).toBeGreaterThan(0);
  await cards.first().click();
  await expect(page.locator(".group-card.selected")).toHaveCount(1);

  // the backend refuses delete_group on built entities
  await page.keyboard.press("Delete");

  // the refusal surfaces as the error chip, with a real message
  const chip = page.getByTestId("sb-error");
  await expect(chip).toBeVisible();
  expect((await chip.innerText()).replace("⚠", "").trim().length).toBeGreaterThan(0);

  // no desync: every card still rendered, selection retained
  await expect(cards).toHaveCount(cardCount);
  await expect(page.locator(".group-card.selected")).toHaveCount(1);

  // click dismisses the chip (no waiting on the auto-clear)
  await chip.click();
  await expect(chip).toBeHidden();

  // leave the app on the map for any spec that might follow
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("map-root")).toBeVisible();
});
