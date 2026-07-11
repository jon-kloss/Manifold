// Phase 1 exit criterion, driven through the real UI against the real core:
// place a factory on the map → claim a node → build the Modular Frame chain in
// the graph → drag the target and watch the chain re-solve live with belt
// saturation coloring → reload and find everything persisted with working undo.

import { test, expect, type Page } from "@playwright/test";

test.describe.configure({ mode: "serial" });

async function addGroup(page: Page, recipeQuery: string, at: { x: number; y: number }) {
  const menu = page.locator(".addgroup-menu input");
  // A click that clears a selection can race the dblclick pair — retry.
  for (let attempt = 0; attempt < 4; attempt++) {
    await page.locator(".react-flow__pane").dblclick({ position: at, delay: 50 });
    try {
      await menu.waitFor({ state: "visible", timeout: 900 });
      break;
    } catch {
      /* retry */
    }
  }
  await menu.fill(recipeQuery);
  await page.locator(".addgroup-item").first().click();
  await page.waitForTimeout(200);
}

async function connect(page: Page, source: string, target: string) {
  const before = await page.locator(".belt-label").count();
  const src = page.locator(`[data-testid="${source}"] .react-flow__handle.source`);
  const dst = page.locator(`[data-testid="${target}"] .react-flow__handle.target`);
  for (let attempt = 0; attempt < 3; attempt++) {
    const a = await src.boundingBox();
    const b = await dst.boundingBox();
    if (!a || !b) throw new Error(`handle not found: ${source} → ${target}`);
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
    await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 10 });
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    await page.mouse.up();
    await page.waitForTimeout(250);
    if ((await page.locator(".belt-label").count()) === before + 1) return;
  }
  throw new Error(`connect failed after retries: ${source} → ${target}`);
}

test("plan the Modular Frame factory end-to-end, offline", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // ---- fresh plan: the first-run card offers three doors, no tour ----
  await expect(page.getByTestId("onboarding")).toBeVisible();
  await expect(page.getByTestId("door-factory")).toBeVisible();
  await expect(page.getByTestId("door-wizard")).toBeVisible();
  await expect(page.getByTestId("door-import")).toBeVisible();
  await expect(page.getByTestId("onboarding")).toContainText("NO TOUR");
  await page.getByTestId("onboard-skip").click();
  await expect(page.getByTestId("onboarding")).not.toBeVisible();

  // ---- place a factory on the map ----
  await page.getByTestId("btn-add-factory").click();
  await page.locator(".map-leaflet").click({ position: { x: 820, y: 700 } });
  await expect(page.getByTestId("summary-drawer")).toBeVisible();

  // rename it
  await page.locator(".drawer-name").click();
  await page.locator(".drawer-name-input").fill("MODULAR WORKS");
  await page.keyboard.press("Enter");
  await expect(page.locator(".pin-chip")).toContainText("MODULAR WORKS");

  // ---- claim an iron node (creates the input port with its ceiling) ----
  await page.keyboard.press("Escape");
  await page.locator(".searchbox input").fill("iron ore");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("node-drawer")).toBeVisible();
  await page.getByTestId("node-drawer").locator("select").nth(1).selectOption("Build_MinerMk2_C");
  await page.getByTestId("btn-claim").click();
  await page.waitForTimeout(200);

  // ---- open the factory graph ----
  await page.locator(".searchbox input").fill("modular works");
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();
  await expect(page.getByTestId("port-in-Desc_OreIron_C")).toBeVisible();

  // ---- output port for Modular Frames ----
  await page.getByRole("button", { name: "+ OUT PORT" }).click();
  await page.locator(".addgroup-menu input").fill("modular frame");
  await page.locator(".addgroup-item").first().click();
  await expect(page.getByTestId("port-out-Desc_ModularFrame_C")).toBeVisible();

  // ---- the six machine groups ----
  // spaced so no dblclick lands on a previously added card even at max zoom
  // (2×: cards render ~500×230 px from their top-left click point)
  await addGroup(page, "iron ingot", { x: 200, y: 140 });
  await addGroup(page, "iron rod", { x: 760, y: 140 });
  await addGroup(page, "screw", { x: 1320, y: 140 });
  await addGroup(page, "iron plate", { x: 200, y: 620 });
  await addGroup(page, "reinforced", { x: 760, y: 620 });
  await addGroup(page, "modular frame", { x: 1320, y: 620 });

  // ---- wire the chain (deselect first so the inspector doesn't cover ports) ----
  await page.keyboard.press("Escape");
  await page.keyboard.press("f"); // frame everything so no card sits off-viewport
  await page.waitForTimeout(500);
  await connect(page, "port-in-Desc_OreIron_C", "group-Recipe_IngotIron_C");
  await connect(page, "group-Recipe_IngotIron_C", "group-Recipe_IronRod_C");
  await connect(page, "group-Recipe_IngotIron_C", "group-Recipe_IronPlate_C");
  await connect(page, "group-Recipe_IronRod_C", "group-Recipe_Screw_C");
  await connect(page, "group-Recipe_IronRod_C", "group-Recipe_ModularFrame_C");
  await connect(page, "group-Recipe_IronPlate_C", "group-Recipe_IronPlateReinforced_C");
  await connect(page, "group-Recipe_Screw_C", "group-Recipe_IronPlateReinforced_C");
  await connect(page, "group-Recipe_IronPlateReinforced_C", "group-Recipe_ModularFrame_C");
  await connect(page, "group-Recipe_ModularFrame_C", "port-out-Desc_ModularFrame_C");
  await expect(page.locator(".belt-label")).toHaveCount(9);

  // ---- upgrade the ore feed belt (Mk.1 at 60/min would bind before the node) ----
  await page.getByTestId("group-Recipe_IngotIron_C").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  await page.getByTestId("inspector").locator("select").first().selectOption("3");
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");

  // ---- drag the target rate: live re-solve with saturation coloring ----
  await page.getByTestId("port-out-Desc_ModularFrame_C").click();
  const slider = page.getByTestId("target-slider");
  await expect(slider).toBeVisible();
  // 120 ore/min ceiling → max 5/min. Drag to 40% ≈ 2/min.
  const box = (await slider.boundingBox())!;
  await page.mouse.move(box.x + 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.4, box.y + box.height / 2, { steps: 10 });
  // mid-drag: projected (italic) values visible before release
  await expect(page.getByTestId("target-value")).toHaveClass(/projected/);
  await page.mouse.up();
  await page.waitForTimeout(400);

  // T1 settled: value upright, solve chip honest
  await expect(page.getByTestId("target-value")).not.toHaveClass(/projected/);
  const target = await page.getByTestId("target-value").innerText();
  const rate = parseFloat(target);
  expect(rate).toBeGreaterThan(1.2);
  expect(rate).toBeLessThanOrEqual(5.0);
  await expect(page.getByTestId("solve-chip")).toContainText("ms");

  // golden chain check via the input port card: ore = 24 × target
  const oreText = await page.getByTestId("port-in-Desc_OreIron_C").locator(".t-data-12").innerText();
  const ore = parseFloat(oreText);
  expect(Math.abs(ore - 24 * rate)).toBeLessThan(0.5);

  // belt saturation coloring: labels show n/cap · %
  await expect(page.locator(".belt-label").first()).toContainText("%");

  // belts are orthogonal runs (edgeLayout), not beziers: no cubic segments
  const edgePath = await page.locator(".react-flow__edge path").first().getAttribute("d");
  expect(edgePath).not.toContain("C");
  expect(edgePath).toMatch(/L /);

  // ---- floors: move the RIP assembler to F1 → chips + lift glyph appear ----
  await page.getByTestId("group-Recipe_IronPlateReinforced_C").click();
  await page.getByTestId("floor-stepper").getByRole("button", { name: "+" }).click();
  await page.waitForTimeout(300);
  await expect(page.getByTestId("floor-chips")).toBeVisible();
  await expect(page.locator(".belt-lift").first()).toBeVisible();
  // floor filter dims off-floor cards
  await page.getByTestId("floor-chips").getByRole("button", { name: "F1" }).click();
  await expect(page.locator('.react-flow__node[style*="opacity: 0.22"]').first()).toBeVisible();
  await page.getByTestId("floor-chips").getByRole("button", { name: "ALL" }).click();

  // ---- AUTO-FLOOR: stages the chain (smelt F0 … final assembly F4), one undo ----
  await page.getByTestId("btn-auto-floor").click();
  await page.waitForTimeout(700);
  await expect(page.getByTestId("floor-chips")).toContainText("F4");
  // smelter is stage 0, final assembler stage 4 — check the card badges
  await expect(page.getByTestId("group-Recipe_IngotIron_C")).toContainText("F0");
  await expect(page.getByTestId("group-Recipe_ModularFrame_C")).toContainText("F4");
  await page.keyboard.press("ControlOrMeta+z"); // one step back to the F1 experiment
  await page.waitForTimeout(400);
  await expect(page.getByTestId("floor-chips")).not.toContainText("F4");

  // ---- STACK FLOORS: cutaway elevation, one undo step ----
  await page.getByTestId("btn-stack-floors").click();
  await page.waitForTimeout(700);
  const p0 = (await page.getByTestId("floor-plate-0").boundingBox())!;
  const p1 = (await page.getByTestId("floor-plate-1").boundingBox())!;
  expect(p1.y + p1.height).toBeLessThanOrEqual(p0.y + 1); // F1 band fully above F0
  await page.keyboard.press("ControlOrMeta+z"); // single undo restores the layout
  await page.waitForTimeout(400);
  const p0b = (await page.getByTestId("floor-plate-0").boundingBox())!;
  const p1b = (await page.getByTestId("floor-plate-1").boundingBox())!;
  expect(p1b.y + p1b.height).toBeGreaterThan(p0b.y + 1); // bands interleave again
  // put it back on F0 (undoable command like any other)
  await page.getByTestId("floor-stepper").getByRole("button", { name: "−" }).click();
  await page.waitForTimeout(300);
  await page.keyboard.press("Escape");

  // ---- hard stop: drag past the ceiling clamps at max and names the constraint ----
  // (re-open the inspector — the floors interlude deselected everything)
  await page.getByTestId("port-out-Desc_ModularFrame_C").click();
  await expect(page.getByTestId("target-slider")).toBeVisible();
  const box2 = (await page.getByTestId("target-slider").boundingBox())!;
  await page.mouse.move(box2.x + 2, box2.y + box2.height / 2);
  await page.mouse.down();
  await page.mouse.move(box2.x + box2.width * 1.05, box2.y + box2.height / 2, { steps: 10 });
  await page.mouse.up();
  await page.waitForTimeout(400);
  await expect(page.getByTestId("binding-strip")).toBeVisible();
  // the Mk.1 screw belt (60/min) binds: 18T = 60 → T = 3.33. Named, with a fix.
  await expect(page.getByTestId("binding-strip")).toContainText("BELT");
  await expect(page.getByTestId("binding-strip")).toContainText("UPGRADE BELT TIER");
  const clamped = parseFloat(await page.getByTestId("target-value").innerText());
  expect(Math.abs(clamped - 60 / 18)).toBeLessThan(0.05);

  // ---- undo includes the solve (counts revert with the rate) ----
  await page.keyboard.press("ControlOrMeta+z");
  await page.waitForTimeout(300);
  const afterUndo = parseFloat(await page.getByTestId("target-value").innerText());
  expect(Math.abs(afterUndo - rate)).toBeLessThan(0.05);

  // ---- reopen: everything persisted (view state returns to the open factory) ----
  await page.reload();
  await expect(page.getByTestId("graph-root")).toBeVisible();
  await expect(page.getByTestId("port-out-Desc_ModularFrame_C")).toBeVisible();
  await page.getByTestId("port-out-Desc_ModularFrame_C").click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  const persisted = parseFloat(await page.getByTestId("target-value").innerText());
  expect(Math.abs(persisted - rate)).toBeLessThan(0.05);
  await expect(page.locator(".belt-label")).toHaveCount(9);

  // undo still works after reopen — the journal lives in the plan file. The
  // last applied edit was the floor reset, so undo resurrects the F1 floor
  // (chips reappear) and redo clears it again; the target rate is untouched.
  await page.keyboard.press("ControlOrMeta+z");
  await expect(page.getByTestId("floor-chips")).toBeVisible();
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(page.getByTestId("floor-chips")).not.toBeVisible();
  const afterReopenRedo = parseFloat(await page.getByTestId("target-value").innerText());
  expect(Math.abs(afterReopenRedo - rate)).toBeLessThan(0.05);

  // ---- logistics: place a splitter from the catalog, then remove it ----
  await page.getByTestId("btn-logistic").click();
  await page.getByTestId("logistic-menu").getByRole("button", { name: "Conveyor Splitter" }).click();
  await expect(page.locator('[data-testid^="junction-splitter-"]')).toBeVisible();
  await page.keyboard.press("Delete");
  await expect(page.locator('[data-testid^="junction-splitter-"]')).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+z"); // undo restores the buildable
  await expect(page.locator('[data-testid^="junction-splitter-"]')).toBeVisible();
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(page.locator('[data-testid^="junction-splitter-"]')).toHaveCount(0);

  // back to the map: pin still there, position intact
  await page.getByTestId("btn-world").click();
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.locator(".pin-chip")).toContainText("MODULAR WORKS");
});

test("A1: refuses gracefully below the 1366×768 floor", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await expect(page.locator(".refuse-card")).toContainText("1366×768");
  await expect(page.locator(".refuse-card .mono")).toContainText("1280×720");
  await page.setViewportSize({ width: 1920, height: 1080 });
  await expect(page.getByTestId("map-root")).toBeVisible();
});
