// W2a exit criterion: refactor/cutover proposals. Drives the real Rust core
// through the dev-bridge: select a running ◆ factory → PLAN REPLACEMENT →
// accept → a ◇ replacement lands beside it carrying `replaces` (REPLACES tether
// + INCOMING/RETIRING pin tags) → the resume dashboard's CUTOVER TIMELINE shows
// BuildNew / Switch / Dismantle in order with a scratch-solved downtime dip
// chip, and a cutover step takes a manual, undoable override.
//
// The ◆ built layer is NEVER mutated by any of this (the refactor only PLANS —
// dismantle is intent, and re-import syncs the eventual teardown; those derived
// completions are covered deterministically by the Rust cutover/session tests,
// which can stage a full re-import that the shared serial e2e plan cannot).

import { test, expect } from "@playwright/test";

import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

// Deterministic map boot — never inherit a dead predecessor's viewState.
test.beforeEach(async ({ request }) => resetView(request));

test("plan a replacement: refactor proposal → cutover timeline + downtime", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/");
  await page.keyboard.press("Escape"); // dismiss any auto-presented dashboard
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });

  // Find a running ◆ built factory that ships iron ingot (raw-sourceable, so the
  // global solver can always re-plan it). Earlier serial specs import Dunarr's
  // built layer, which contains an IRON INGOT WORKS.
  type Hydrate = {
    plan: {
      factories: Record<string, { id: string; name: string; status: string; ports: string[]; replaces?: string | null }>;
      ports: Record<string, { direction: string; item: string }>;
    };
  };
  const hydrate = async (): Promise<Hydrate> => (await page.request.get("/api/hydrate")).json();
  const pickOld = (h: Hydrate) =>
    Object.values(h.plan.factories).find(
      (f) =>
        f.status === "built" &&
        f.ports.some((pid) => h.plan.ports[pid]?.direction === "out" && h.plan.ports[pid]?.item === "Desc_IronIngot_C"),
    );

  let old = pickOld(await hydrate());
  expect(old, "a ◆ built iron-ingot factory from an earlier import").toBeTruthy();
  const oldId = old!.id;
  const oldName = old!.name;

  // Select the ◆ factory via search → its summary drawer opens.
  await page.locator(".searchbox input").fill(oldName.toLowerCase());
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("summary-drawer")).toBeVisible();

  // PLAN REPLACEMENT → the real solver drafts a Refactor proposal and opens it.
  await page.getByTestId("btn-plan-replacement").click();
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 30_000 });
  await page.getByTestId("btn-accept-proposal").click();
  await expect(page.getByTestId("proposal-review")).not.toBeVisible();

  // The ◆ old factory is untouched; a NEW ◇ factory now carries replaces → old.
  const after = await hydrate();
  const newFactory = Object.values(after.plan.factories).find((f) => f.replaces === oldId);
  expect(newFactory, "a ◇ replacement carrying replaces").toBeTruthy();
  expect(after.plan.factories[oldId].status, "◆ built layer never mutated").toBe("built");

  // Pin tags: the old ◆ reads RETIRING, the new ◇ reads INCOMING (chips may be
  // decluttered off-screen, so assert they are attached to the DOM).
  await expect(page.locator(".pin-tag.retiring").first()).toBeAttached();
  await expect(page.locator(".pin-tag.incoming").first()).toBeAttached();

  // Open the resume dashboard → the CUTOVER TIMELINE section renders the phases
  // in order with a scratch-solved downtime dip.
  await page.keyboard.press("h");
  await expect(page.getByTestId("dashboard")).toBeVisible();
  const timeline = page.getByTestId("cutover-timeline");
  await expect(timeline).toBeVisible();
  await expect(timeline).toContainText("CUTOVER TIMELINE");
  await expect(timeline.getByTestId("cutover-card").first()).toContainText(oldName.toUpperCase());
  // BuildNew → Switch → Dismantle phase headers, in DOM (source) order.
  await expect(page.getByTestId("cutover-phase-build_new").first()).toBeVisible();
  await expect(page.getByTestId("cutover-phase-switch").first()).toBeVisible();
  await expect(page.getByTestId("cutover-phase-dismantle").first()).toBeVisible();
  // The downtime is scratch-solved on demand — a dip chip appears (old down while
  // the switch happens ⇒ iron-ingot output dips below baseline).
  await expect(page.getByTestId("downtime-dip").first()).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("downtime-dip").first()).toContainText("(est)");

  // Mark a cutover step done → one manual, undoable override (badge appears).
  await expect(page.locator('[data-testid="cutover-override"]')).toHaveCount(0);
  await page.locator('[data-testid="cutover-step"] input:not(:checked)').first().click();
  await expect(page.locator('[data-testid="cutover-override"]').first()).toBeVisible();
  // ⌘Z reverts the override in one step.
  await page.keyboard.press("Control+z");
  await expect(page.locator('[data-testid="cutover-override"]')).toHaveCount(0);

  // Dismiss → the restored map shows through unchanged (this is the last serial
  // spec; the accepted ◇ replacement is left in the shared plan intentionally).
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("dashboard")).toBeHidden();
  await expect(page.getByTestId("map-root")).toBeVisible();
});
