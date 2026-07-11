// Phase 3 exit criterion: "plan a supply chain" produces a reviewable,
// partially-acceptable proposal — through the real wizard UI against the real
// global solver. Also covers undo-of-accept, FIX WITH SOLVER prefill, and
// priority switches on the phase-2 power grid.

import { test, expect } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("wizard → reviewable proposal → partial accept → undo", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  const factoryCount = async () =>
    Number(/(\d+) FACTORIES/.exec((await page.locator(".statusbar").innerText()) ?? "")?.[1] ?? -1);
  const before = await factoryCount();

  // P opens the wizard; goal: 25 iron plates/min (nothing produces plates yet)
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.selectOption('[data-testid="wizard-item"]', "Desc_IronPlate_C");
  await page.fill('[data-testid="wizard-rate"]', "25");
  await page.click('[data-testid="wizard-solve"]');

  // solve streams a log then hands off to the review surface
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("proposal-review")).toContainText("PRODUCE IRON PLATE AT 25.0/MIN");
  await expect(page.getByTestId("proposal-review")).toContainText("IRON PLATE WORKS — NEW");
  await expect(page.getByTestId("goal-check")).toContainText("25/25 ✓");
  const items = await page.getByTestId("proposal-item").count();
  expect(items).toBeGreaterThanOrEqual(2);

  // partial accept: SPACE on the first CLAIM/ROUTE row strikes it + recomputes
  const acceptBtn = page.getByTestId("btn-accept-proposal");
  await expect(acceptBtn).toContainText(`ACCEPT ${items} AS PLANNED`);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press(" ");
  await expect(page.locator(".prop-row.excluded").first()).toBeVisible();
  await expect(acceptBtn).not.toContainText(`ACCEPT ${items} AS PLANNED`);
  // re-include: dependents stay off until re-checked themselves (the cascade
  // never guesses) — click each remaining excluded checkbox
  await page.keyboard.press(" ");
  while ((await page.locator(".prop-row.excluded").count()) > 0) {
    await page.locator(".prop-row.excluded input[type=checkbox]").first().click();
    await page.waitForTimeout(150);
  }
  await expect(page.locator(".prop-row.excluded")).toHaveCount(0);
  await acceptBtn.click();

  // review closes; the site exists as a ◇ pin; ONE undo removes it all
  await expect(page.getByTestId("proposal-review")).not.toBeVisible();
  await expect(page.locator(".pin-chip", { hasText: "IRON PLATE WORKS" })).toBeVisible();
  expect(await factoryCount()).toBe(before + 1);
  await page.keyboard.press("Control+z");
  await expect(page.locator(".pin-chip", { hasText: "IRON PLATE WORKS" })).not.toBeVisible();
  expect(await factoryCount()).toBe(before);
  await page.keyboard.press("Control+Shift+z"); // redo — keep the site for later specs
  await expect(page.locator(".pin-chip", { hasText: "IRON PLATE WORKS" })).toBeVisible();
});

test("audit FIX WITH SOLVER pre-fills the wizard goal", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
  await page.locator(".audit-tab", { hasText: "DEFICITS" }).click();
  // phase-2 left an ingot starvation; target that row's FIX chip explicitly
  const row = page.locator('.audit-row:has-text("Iron Ingot")').first();
  await expect(row).toBeVisible();
  await row.locator(".chip", { hasText: "FIX WITH SOLVER" }).click();
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await expect(page.getByTestId("wizard-item")).toHaveValue("Desc_IronIngot_C");
  await page.keyboard.press("Escape");
});

test("priority switch on the grid: sheds-at + brownout sim", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  // select the COAL PLANT → ROD CITY power line by clicking its midpoint
  const pin = async (name: string) => {
    const box = await page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`).boundingBox();
    if (!box) throw new Error(`pin ${name}`);
    return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  };
  const a = await pin("COAL PLANT");
  const b = await pin("ROD CITY");
  await page.mouse.click((a.x + b.x) / 2, (a.y + b.y) / 2);
  await expect(page.getByTestId("route-drawer")).toContainText("POWER LINE");

  // add a switch → its inspector opens with a live sheds-at threshold
  await page.getByTestId("btn-add-switch").click();
  await expect(page.getByTestId("switch-drawer")).toBeVisible();
  await expect(page.getByTestId("switch-drawer")).toContainText("Sheds at");
  await page.selectOption('[data-testid="switch-priority"]', "8");
  await expect(page.getByTestId("switch-drawer")).toContainText("Brownout sim: next shed P8");

  // audit POWER tab gains PRIORITY + SHEDS AT + the synthetic sim row
  await page.keyboard.press("Escape");
  await page.keyboard.press("Tab");
  await page.locator(".audit-tab", { hasText: "POWER" }).click();
  await expect(page.getByTestId("brownout-row")).toContainText("BROWNOUT SIM — next shed: P8");
  await expect(page.getByTestId("switch-row")).toContainText("PRIORITY P8");
  await expect(page.getByTestId("switch-row")).toContainText("SHEDS AT");
  await page.keyboard.press("Tab");

  // clean up: delete the switch through its drawer
  const sw = page.locator("canvas").first();
  void sw;
  const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
  await page.mouse.click(mid.x, mid.y); // switch sits at the line midpoint
  await expect(page.getByTestId("switch-drawer")).toBeVisible();
  await page.getByRole("button", { name: "DELETE SWITCH" }).click();
  await expect(page.getByTestId("switch-drawer")).not.toBeVisible();
});
