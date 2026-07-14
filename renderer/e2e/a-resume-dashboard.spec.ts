// T6 — resume-dashboard regression (R1). A build-from-scratch plan opens EMPTY
// first; the once-per-plan `resumeSeen` flag must NOT be spent on that empty
// open, or the dashboard would never auto-present once work exists.
//
// Determinism: this spec runs FIRST (its filename sorts before every other e2e
// file), so it sees the freshly-wiped shared plan the config removes before the
// dev-bridge boots — a genuinely empty plan with `resumeSeen` unpersisted, which
// is exactly the precondition the regression needs and which no later spec can
// reproduce (they all accumulate factories + burn the flag). It leaves the plan
// non-empty for the specs that follow, which self-seed and tolerate that.

import { test, expect } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("empty → build → reopen: resume dashboard auto-presents (R1)", async ({ page }) => {
  // ---- 1. EMPTY first open: no dashboard, no burned flag ----
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });
  // Empty plan → Onboarding, never the resume dashboard. Under the old code the
  // auto-present effect burned `resumeSeen` right here (unconditionally, above
  // the present gate); the fix only spends it inside the present branch.
  await expect(page.getByTestId("onboarding")).toBeVisible();
  await expect(page.getByTestId("dashboard")).toBeHidden();

  // ---- 2. add work: draft + accept a proposal → a ◇ planned build step ----
  await page.getByTestId("door-wizard").click();
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.selectOption('[data-testid="wizard-item"]', "Desc_IronPlate_C");
  await page.fill('[data-testid="wizard-rate"]', "10");
  await page.click('[data-testid="wizard-solve"]');
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 15_000 });
  await page.getByTestId("btn-accept-proposal").click();
  await expect(page.getByTestId("proposal-review")).not.toBeVisible();

  // The empty open must NOT have persisted `resumeSeen`. By now the wizard round
  // trip has elapsed, so any (buggy) write from step 1 has long since landed —
  // asserting its absence here is deterministic, not a race.
  const vs = (await (await page.request.get("/api/hydrate")).json()).viewState ?? {};
  expect(vs.resumeSeen ?? false).toBe(false);

  // ---- 3. reload with work present → the dashboard AUTO-presents ----
  // No manual resumeSeen reset (unlike w1c): the flag was never spent, so a
  // plain reload must greet. Under the old code it would stay hidden forever.
  await page.reload();
  await expect(page.getByTestId("dashboard")).toBeVisible({ timeout: 15_000 });

  // dismiss reveals the restored map, unchanged (Escape consumes here — R6).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("dashboard")).toBeHidden();
  await expect(page.getByTestId("map-root")).toBeVisible();

  // ---- cleanup: undo every step back to the pristine empty plan (undo is the
  // same proven-safe path ⌘Z uses — the draft proposal and the accept are both
  // undoable), then clear viewState. This restores the exact post-wipe state so
  // the specs that follow — exit-criterion runs next and asserts a fresh
  // Onboarding on an EMPTY plan — behave as if this spec never seeded anything. ----
  for (let i = 0; i < 50; i++) {
    const h = await (await page.request.get("/api/hydrate")).json();
    if (!h.canUndo) break;
    await page.request.post("/api/undo");
  }
  await page.request.post("/api/view", { data: JSON.stringify({}) });
  const after = (await (await page.request.get("/api/hydrate")).json()).plan;
  expect(Object.keys(after.factories).length).toBe(0);
  expect(Object.keys(after.proposals).length).toBe(0);
});
