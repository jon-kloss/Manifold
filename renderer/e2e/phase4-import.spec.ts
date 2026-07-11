// Phase 4 exit criterion: import Dunarr-076; drift renders in DIFF.
// Also: rail route math block through the real inspector.

import { test, expect, type Page } from "@playwright/test";

test.describe.configure({ mode: "serial" });

const SAVES = "/home/user/Conveyancer/fixtures/saves";

async function importSave(page: Page, file: string) {
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByTestId("btn-import").click(),
  ]);
  await chooser.setFiles(`${SAVES}/${file}`);
  await expect(page.getByTestId("import-preview")).toBeVisible({ timeout: 120_000 });
  await page.getByTestId("btn-import-run").click();
}

test("import Dunarr-076 as the built layer; drift renders in DIFF", async ({ page }) => {
  test.setTimeout(300_000); // two .sav parses in a cold worker
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  const factoryCount = async () =>
    Number(/(\d+) FACTORIES/.exec(await page.locator(".statusbar").innerText())?.[1] ?? -1);
  const before = await factoryCount();

  // ---- first import writes the ◆ built layer ----
  await importSave(page, "Dunarr-076.sav");
  await expect(page.getByTestId("import-done")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("import-done")).toContainText("13 factories · 867 machines imported as ◆ BUILT");
  await expect(page.getByTestId("import-done")).toContainText("quarantined");
  await page.locator(".wizard-foot .btn-primary").click();
  expect(await factoryCount()).toBe(before + 13);
  await expect(page.locator(".pin-chip", { hasText: "IRON INGOT WORKS 1" })).toBeVisible();

  // one undo removes the entire import; redo restores it
  await page.keyboard.press("Control+z");
  await expect.poll(factoryCount).toBe(before);
  await page.keyboard.press("Control+Shift+z");
  await expect.poll(factoryCount).toBe(before + 13);

  // ---- DIFF: plan-vs-built drift (◇ plan ahead of the game) ----
  await page.keyboard.press("Tab");
  await page.locator(".audit-tab", { hasText: "PLAN DRIFT" }).click();
  await expect(page.getByTestId("plan-drift-row")).toContainText("planned, not yet built in-game");
  await page.keyboard.press("Tab");

  // ---- identical re-import: never writes, reports in sync ----
  await importSave(page, "Dunarr-076.sav");
  await expect(page.getByTestId("import-done")).toBeVisible({ timeout: 60_000 });
  await expect(page.getByTestId("import-done")).toContainText("IN SYNC");
  await page.locator(".wizard-foot .btn-primary").click();

  // ---- a different world's save: pure game drift → review + DIFF rows ----
  await importSave(page, "269.sav");
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 120_000 });
  await expect(page.locator(".prop-banner")).toContainText("RE-IMPORT 269");
  await expect(page.locator(".prop-banner")).toContainText("SAVE RE-IMPORT");
  const driftItems = await page.getByTestId("proposal-item").count();
  expect(driftItems).toBeGreaterThan(5);
  // exit WITHOUT accepting — re-imports never write
  await page.getByTestId("btn-exit-review").click();
  expect(await factoryCount()).toBe(before + 13);

  // the DIFF tab carries the game-drift rows with a REVIEW action
  await page.keyboard.press("Tab");
  await page.locator(".audit-tab", { hasText: "PLAN DRIFT" }).click();
  expect(await page.getByTestId("drift-row").count()).toBe(driftItems);
  await expect(page.getByTestId("drift-row").first()).toContainText("GAME DRIFT");
  await page.keyboard.press("Tab");
});

test("rail route: the math block is the product", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  // right-drag IRON PLATE WORKS (phase-3 site, unbound OUT) → DEPOT SOUTH
  const pin = async (name: string) => {
    const box = await page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`).boundingBox();
    if (!box) throw new Error(`pin ${name}`);
    return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  };
  const a = await pin("IRON PLATE WORKS");
  const b = await pin("DEPOT SOUTH");
  await page.mouse.move(a.x, a.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(b.x, b.y, { steps: 8 });
  await page.mouse.up({ button: "right" });
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await page.selectOption('[data-testid="popover-transport"]', "rail");
  await page.getByTestId("btn-route-confirm").click();

  // the inspector opens on the rail route with the visible math block
  await expect(page.getByTestId("route-drawer")).toContainText("RAIL ROUTE");
  const math = page.getByTestId("math-block");
  await expect(math).toBeVisible();
  await expect(math).toContainText("ROUND TRIP");
  await expect(math).toContainText("HEADWAY");
  await expect(math).toContainText("RTT");
  await expect(math).toContainText("THROUGHPUT");
  await expect(math).toContainText("DEMAND");

  // +1 consist doubles throughput (1 → 2)
  const throughput = async () =>
    parseFloat(/THROUGHPUT[\s\S]*?([\d,.]+)\/min/.exec((await math.innerText()).replace(/\n/g, " "))?.[1] ?? "0");
  const t1 = await throughput();
  expect(t1).toBeGreaterThan(0);
  await page.getByTestId("btn-add-consist").click();
  await expect(page.getByTestId("consist-row")).toContainText("2×");
  const t2 = await throughput();
  expect(t2).toBeGreaterThan(t1 * 1.9);

  // ---- the rail route is actually drawn on the map: deselect, then click the
  // line itself. hitTestRoute only sees routes in the canvas data, so this
  // fails if rail routes are filtered out of the drawn set. ----
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("route-drawer")).toBeHidden();
  const a2 = await pin("IRON PLATE WORKS");
  const b2 = await pin("DEPOT SOUTH");
  // Neighbouring pin chips (DOM) can cover parts of the line; probe along the
  // segment for a spot where the map itself would receive the click, so the
  // hit lands on the canvas line — not on a pin.
  const spot = await page.evaluate(([ax, ay, bx, by]) => {
    for (const t of [0.5, 0.65, 0.35, 0.75, 0.25, 0.6, 0.4]) {
      const x = ax + (bx - ax) * t;
      const y = ay + (by - ay) * t;
      if (document.elementFromPoint(x, y)?.classList.contains("leaflet-container")) return { x, y };
    }
    return null;
  }, [a2.x, a2.y, b2.x, b2.y]);
  if (!spot) throw new Error("no unobstructed point on the rail segment");
  await page.mouse.click(spot.x, spot.y);
  await expect(page.getByTestId("route-drawer")).toBeVisible();
  await expect(page.getByTestId("route-drawer")).toContainText("RAIL ROUTE");

  // ---- and audited: the SATURATION tab lists it as ROUTE · RAIL ----
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
  await expect(page.getByTestId("audit-drawer")).toContainText("IRON PLATE WORKS ⟶ DEPOT SOUTH");
  await expect(page.getByTestId("audit-drawer")).toContainText("ROUTE · RAIL");
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeHidden();

  // swapping to drone rewires the same binding with drone math
  await page.selectOption('[data-testid="route-kind-select"]', "drone");
  await expect(page.getByTestId("route-drawer")).toContainText("DRONE ROUTE");
  await expect(page.getByTestId("math-block")).toContainText("BATTERIES");
  // clean up: back to belt so downstream specs see familiar state
  await page.selectOption('[data-testid="route-kind-select"]', "belt");
  await expect(page.getByTestId("route-drawer")).toContainText("BELT ROUTE");
  await page.keyboard.press("Escape");
});
