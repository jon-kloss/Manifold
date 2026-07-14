// W1c exit criterion: the build queue is a DERIVED projection and the
// session-resume dashboard auto-presents once per open. Drives the real Rust
// core through the dev-bridge: draft/accept a proposal → a ◇ Pending step
// appears in the dashboard → a manual checkbox writes an undoable override →
// ⌘Z reverts it → reload auto-presents the dashboard, which dismisses back to
// the map. (The derived Done-flip on a built twin and the override auto-dissolve
// on re-import are covered deterministically by the Rust buildqueue/session
// tests — a built twin at a planned site can't be staged reliably in the shared
// serial e2e plan.)

import { test, expect } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("derived build queue + resume dashboard: override, undo, auto-present", async ({ page }) => {
  await page.goto("/");
  // Settle to the map: one Escape dismisses any auto-presented dashboard AND
  // exits any factory view a prior serial spec left open (GraphView → map).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("dashboard")).toBeHidden();

  // draft + accept a proposal → guarantees a ◇ planned build step
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.selectOption('[data-testid="wizard-item"]', "Desc_IronPlate_C");
  await page.fill('[data-testid="wizard-rate"]', "10");
  await page.click('[data-testid="wizard-solve"]');
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("btn-accept-proposal").click();
  await expect(page.getByTestId("proposal-review")).not.toBeVisible();

  // H opens the resume dashboard; a ◇ Pending step is present and unchecked
  await page.keyboard.press("h");
  await expect(page.getByTestId("dashboard")).toBeVisible();
  const steps = page.locator('[data-testid="build-step"]');
  expect(await steps.count()).toBeGreaterThan(0);
  await expect(page.locator('[data-testid="step-override"]')).toHaveCount(0);

  // check the first Pending step → one manual override (badge), undoable. The
  // checkbox is controlled by derived `done` (updated after the async dispatch),
  // so click and assert the override lands rather than using .check()'s
  // synchronous state check.
  await page.locator('[data-testid="build-step"] input:not(:checked)').first().click();
  await expect(page.locator('[data-testid="step-override"]')).toHaveCount(1);

  // ⌘Z reverts the override in one step
  await page.keyboard.press("Control+z");
  await expect(page.locator('[data-testid="step-override"]')).toHaveCount(0);

  // dismiss → the restored map shows through unchanged
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("dashboard")).toBeHidden();
  await expect(page.getByTestId("map-root")).toBeVisible();

  // the StatusBar resume chip reopens it on demand
  await page.getByTestId("sb-resume").click();
  await expect(page.getByTestId("dashboard")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("dashboard")).toBeHidden();

  // auto-present: clear the persisted "seen" flag so the next open re-greets,
  // then reload with a non-empty queue → the dashboard auto-presents
  const vs = (await (await page.request.get("/api/hydrate")).json()).viewState ?? {};
  await page.request.post("/api/view", { data: JSON.stringify({ ...vs, resumeSeen: false }) });
  await page.reload();
  await expect(page.getByTestId("dashboard")).toBeVisible({ timeout: 15_000 });
  // dismiss reveals the restored map, unchanged
  await page.getByRole("button", { name: "Dismiss" }).click();
  await expect(page.getByTestId("dashboard")).toBeHidden();
  await expect(page.getByTestId("map-root")).toBeVisible();
});
