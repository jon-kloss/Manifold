// Phase 3 exit criterion: "plan a supply chain" produces a reviewable,
// partially-acceptable proposal — through the real wizard UI against the real
// global solver. Also covers undo-of-accept, FIX WITH SOLVER prefill, and
// priority switches on the phase-2 power grid.

import { test, expect } from "@playwright/test";

import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

// Deterministic map boot — never inherit a dead predecessor's viewState.
test.beforeEach(async ({ request }) => resetView(request));

test("wizard → reviewable proposal → partial accept → undo", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  const factoryCount = async () =>
    Number(/(\d+) FACTORIES/.exec((await page.locator(".statusbar").innerText()) ?? "")?.[1] ?? -1);
  const before = await factoryCount();

  // P opens the wizard; goal: 25 iron plates/min (nothing produces plates yet)
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.getByTestId("wizard-item").fill("iron plate");
  await page.getByTestId("wizard-item-option").first().click();
  await page.fill('[data-testid="wizard-rate"]', "25");
  await page.click('[data-testid="wizard-solve"]');

  // solve streams a log then hands off to the review surface
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("proposal-review")).toContainText("PRODUCE IRON PLATE AT 25.0/MIN");
  await expect(page.getByTestId("proposal-review")).toContainText("IRON PLATE WORKS — NEW");
  await expect(page.getByTestId("goal-check")).toContainText("25/25 ✓");
  const items = await page.getByTestId("proposal-item").count();
  expect(items).toBeGreaterThanOrEqual(2);

  // power-forward: the per-circuit banner is pinned above the footer — a grid
  // line with a live headroom figure, and a generator-sizing line — both
  // visible without scrolling the change list
  await expect(page.getByTestId("proposal-power-line").first()).toContainText("headroom");
  await expect(page.getByTestId("proposal-gen-line").first()).toBeVisible();
  await expect(page.getByTestId("proposal-gen-line").first()).toContainText("generation");

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
  await expect(page.getByTestId("wizard-item")).toHaveValue("Iron Ingot");
  await page.keyboard.press("Escape");
});

test("priority switch on the grid: sheds-at + brownout sim", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  // select the COAL PLANT → ROD CITY power line by clicking its midpoint
  const pin = async (name: string) => {
    // poll: a one-shot boundingBox races map init / zoom animation
    const loc = page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`);
    let box = null;
    for (let i = 0; i < 25 && !box; i++) {
      box = await loc.boundingBox().catch(() => null);
      if (!box) await page.waitForTimeout(200);
    }
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

test("total-quantity goal: milestone ladder + carried proposal chip", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // P opens the wizard; pick a craftable item + rate
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.getByTestId("wizard-item").fill("iron plate");
  await page.getByTestId("wizard-item-option").first().click();
  await page.fill('[data-testid="wizard-rate"]', "8");

  // toggle TOTAL-QUANTITY GOAL on and set the huge total the game hands out
  await page.getByTestId("wizard-total-toggle").check();
  await page.fill('[data-testid="wizard-total"]', "2500");

  // the ladder computes time-at-rate purely in UI (2500 / 8 = 5h 12m)
  await expect(page.getByTestId("wizard-ladder")).toBeVisible();
  await expect(page.getByTestId("wizard-ladder")).toContainText("5h 12m");

  await page.click('[data-testid="wizard-solve"]');

  // the proposal carries the milestone: a static chip with the thousands-
  // separated total, the item, and the ETA — under the title
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 10_000 });
  const chip = page.getByTestId("proposal-milestone");
  await expect(chip).toBeVisible();
  await expect(chip).toContainText("2,500");
  await expect(chip).toContainText("IRON PLATE");
  await expect(chip).toContainText("5h 12m");

  // end clean: exit review WITHOUT accepting — leave no proposal open and no
  // new factory in the shared serial state
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("proposal-review")).not.toBeVisible();
});

test("combobox keyboard: Enter picks the option, Escape peels the list first", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();

  // ARIA contract: closed → not expanded; focus opens the list
  const input = page.getByTestId("wizard-item");
  await expect(input).toHaveAttribute("aria-expanded", "false");
  await input.click();
  await expect(input).toHaveAttribute("aria-expanded", "true");
  await input.fill("iron rod");
  await page.keyboard.press("ArrowDown");

  // aria-activedescendant points at the highlighted option's real element
  const activeId = await input.getAttribute("aria-activedescendant");
  expect(activeId).toBeTruthy();
  const activeOpt = page.locator(`[id="${activeId}"]`);
  await expect(activeOpt).toHaveAttribute("role", "option");
  await expect(activeOpt).toHaveAttribute("aria-selected", "true");
  // the option's name span (the sibling item-chip monogram is aria-hidden noise)
  const pickedName = (await activeOpt.locator("span:not(.item-chip)").innerText()).trim();

  // Enter picks the highlighted option — it must NOT double as the wizard's
  // solve key (that would solve for the stale previously-selected item)
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await expect(page.getByTestId("wizard-solve")).toBeVisible(); // still step 1
  await expect(input).toHaveValue(pickedName);
  await expect(input).toHaveAttribute("aria-expanded", "false");

  // reopen the list: Escape dismisses ONLY the list, the wizard survives
  await input.click();
  await expect(page.locator(".item-combo-list")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".item-combo-list")).not.toBeVisible();
  await expect(page.getByTestId("wizard-modal")).toBeVisible();

  // with the list closed, Escape closes the wizard — back where we started
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("wizard-modal")).not.toBeVisible();
});
