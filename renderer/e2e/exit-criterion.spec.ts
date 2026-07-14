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
  // React Flow's synthetic drag-to-connect is the flakiest op in the suite on a
  // loaded CI runner: the handle must be scrolled into view AND hovered before
  // it accepts a connection start, the drop must pause on the target so RF's
  // pointer-over fires, and the whole gesture can still miss under contention.
  // Retry generously with a neutral-position reset between attempts — a stuck
  // half-drag from a missed attempt otherwise swallows the next one.
  await src.waitFor({ state: "visible" });
  await dst.waitFor({ state: "visible" });
  for (let attempt = 0; attempt < 8; attempt++) {
    // Re-scroll EVERY attempt: each landed belt adds labels and shifts layout,
    // so coordinates measured before the previous connect go stale mid-chain
    // (CI reproduced a miss on belt 3 of 7 that never happens locally).
    await src.scrollIntoViewIfNeeded();
    await dst.scrollIntoViewIfNeeded();
    const a = await src.boundingBox();
    const b = await dst.boundingBox();
    if (!a || !b) throw new Error(`handle not found: ${source} → ${target}`);
    const sx = a.x + a.width / 2;
    const sy = a.y + a.height / 2;
    const tx = b.x + b.width / 2;
    const ty = b.y + b.height / 2;
    await page.mouse.move(sx, sy);
    await src.hover(); // arm the handle so RF marks it connectable
    await page.mouse.down();
    await page.mouse.move(sx, sy); // tiny wiggle to register drag start
    await page.mouse.move(tx, ty, { steps: 16 });
    await page.mouse.move(tx, ty); // settle on target so pointer-over fires
    await page.waitForTimeout(60 + attempt * 40);
    await page.mouse.up();
    await page.waitForTimeout(400 + attempt * 100);
    if ((await page.locator(".belt-label").count()) === before + 1) return;
    // Reset any half-committed drag before retrying, with growing backoff so a
    // contended runner gets breathing room instead of six identical fast misses.
    await page.mouse.up().catch(() => {});
    await page.mouse.move(sx, sy - 120);
    await page.waitForTimeout(150 + attempt * 100);
  }
  throw new Error(`connect failed after retries: ${source} → ${target}`);
}

test("plan the Modular Frame factory end-to-end, offline", async ({ page }) => {
  // The suite's longest UI-gesture spec (~40 gestures with retry loops): the
  // default 60s budget fits a warm dev box but not a cold contended CI runner —
  // CI burned the whole budget before the first belt and cascaded the serial
  // suite. Same honest-budget treatment phase4 gives its save parses.
  test.setTimeout(180_000);
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
  await page.locator(".drawer-name-input").fill("TYPO NAME");
  // M23 regression: ⌘Z while typing must NOT undo the plan (factory creation
  // is the only undoable command at this point — the pin would vanish).
  await page.keyboard.press("ControlOrMeta+z");
  await expect(page.locator(".pin-wrap")).toHaveCount(1);
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
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

  // empty factory teaches the add gesture; + MACHINE opens the same menu
  await expect(page.getByTestId("graph-empty-hint")).toBeVisible();
  await page.getByTestId("btn-add-machine").click();
  await expect(page.locator(".addgroup-menu input")).toBeVisible();
  // generators are placeable like any machine: burn recipes carry an MW tag
  await page.locator(".addgroup-menu input").fill("coal");
  await expect(page.locator(".addgroup-item").first()).toContainText("MW");
  await page.keyboard.press("Escape");

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
  await expect(page.getByTestId("graph-empty-hint")).toBeHidden();

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
  // ...and numerically live: the WASM T0 projection must track the drag before
  // release. A dead projection leaves the stale pre-drag value here (regression
  // guard for the Ok/ES-Map serialization bug). Poll — the solve lands on a
  // later frame than the mousemove.
  await expect
    .poll(async () => parseFloat(await page.getByTestId("target-value").innerText()))
    .toBeGreaterThan(1.2);
  const midTarget = parseFloat(await page.getByTestId("target-value").innerText());
  expect(midTarget).toBeLessThanOrEqual(5.0);
  // and the projected chain propagates upstream mid-drag: ore card = 24 × target
  await expect
    .poll(async () =>
      parseFloat(await page.getByTestId("port-in-Desc_OreIron_C").locator(".t-data-12").innerText()),
    )
    .toBeCloseTo(24 * midTarget, 0);
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
  // R1: the once-per-plan resume dashboard auto-presents on THIS first reload
  // with a non-empty build queue — the empty first open (above) no longer wrongly
  // spends the flag. Dismiss it (Escape consumes — R6) to reach the factory view.
  await expect(page.getByTestId("dashboard")).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("dashboard")).toBeHidden();
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

test("A1: small viewports degrade to the overlay layout, never a dead end", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.locator(".app-frame")).toHaveAttribute("data-layout", "overlay");
  await page.setViewportSize({ width: 1920, height: 1080 });
  await expect(page.locator(".app-frame")).toHaveAttribute("data-layout", "reference");
});
