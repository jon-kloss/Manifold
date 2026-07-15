// Phase 5 exit criterion, mechanized: the ambient advisor is not naggy —
// cards fire once per newly-armed condition, dismiss mutes the rule for good,
// pause silences everything, and its loudest voice is a badge count. Chat
// intents become reviewable proposals through the solver, never edits.

import { test, expect } from "@playwright/test";

import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

// Deterministic map boot — never inherit a dead predecessor's viewState.
test.beforeEach(async ({ request }) => resetView(request));

test("advisor: badge, provenance, dismiss-mutes, pause", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // earlier specs left real deficits — the advisor noticed, quietly
  await expect(page.getByTestId("advisor-badge")).toBeVisible();
  await page.keyboard.press("a");
  await expect(page.getByTestId("advisor-panel")).toBeVisible();

  // cards carry SAW/RULE provenance and never-edit microcopy
  const card = page.getByTestId("advisor-card").first();
  await expect(card).toBeVisible();
  await expect(card).toContainText("SAW:");
  await expect(card).toContainText("RULE:");
  await expect(page.getByTestId("advisor-panel")).toContainText(
    "The advisor never edits your plan",
  );
  await expect(page.getByTestId("advisor-panel")).toContainText("AI OFFLINE");

  // dismiss = mute the rule (persisted); the muted chip appears
  const rule = (await card.innerText()).match(/RULE: (\w+)/)?.[1] ?? "";
  const before = await page.getByTestId("advisor-card").count();
  await card.getByTestId("card-dismiss").click();
  await expect(page.getByTestId(`unmute-${rule}`)).toBeVisible();
  expect(await page.getByTestId("advisor-card").count()).toBeLessThan(before);

  // unmute brings the rule back for FUTURE events (no card resurrection)
  const afterDismiss = await page.getByTestId("advisor-card").count();
  await page.getByTestId(`unmute-${rule}`).click();
  await expect(page.getByTestId(`unmute-${rule}`)).not.toBeVisible();
  expect(await page.getByTestId("advisor-card").count()).toBe(afterDismiss);

  // pause flips the ambient chip
  await page.getByTestId("advisor-pause").click();
  await expect(page.getByTestId("advisor-pause")).toContainText("PAUSED");
  await page.getByTestId("advisor-pause").click();
  await expect(page.getByTestId("advisor-pause")).toContainText("AMBIENT");
  await page.keyboard.press("a");
  await expect(page.getByTestId("advisor-panel")).not.toBeVisible();
});

test("chat: context is viewable, intents become proposals", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("a");
  await page.locator(".audit-tab", { hasText: "CHAT" }).click();

  // the exact payload a model would see is user-viewable, size shown
  await page.getByTestId("btn-view-context").click();
  await expect(page.getByTestId("context-json")).toContainText('"scope"');

  // a status question answers with a causal block from derived state
  await page.getByTestId("chat-input").fill("how is power?");
  await page.getByTestId("chat-send").click();
  await expect(page.getByTestId("chat-answer").last()).toContainText("MW", { timeout: 15_000 });
  await expect(page.getByTestId("chat-answer").last()).toContainText("ENGINE: OFFLINE");

  // proposal_intent: drafted through the global solver, reviewed like any other
  await page.getByTestId("chat-input").fill("produce Iron Rod at 10/min");
  await page.getByTestId("chat-send").click();
  await expect(page.getByTestId("chat-review-proposal")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("chat-answer").last()).toContainText("Nothing applies until you review");
  await page.getByTestId("chat-review-proposal").click();
  await expect(page.getByTestId("proposal-review")).toBeVisible();
  await expect(page.locator(".prop-banner")).toContainText("CHAT INTENT");
  // never an edit: exit without accepting
  await page.getByTestId("btn-exit-review").click();
});

test("style guides: manual creation + factory theme link", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("f");
  await page.waitForTimeout(400);
  await page.locator(".pin-chip", { hasText: "INGOT POINT" }).click();
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  await page.getByTestId("btn-new-guide").click();
  await page.getByTestId("theme-select").selectOption({ label: "GUIDE 1" });
  await expect(page.getByTestId("theme-select")).toHaveValue(/./);
  // undo unlinks (theme set is an ordinary undoable command)
  await page.keyboard.press("Control+z");
  await expect(page.getByTestId("theme-select")).toHaveValue("");
});

test("escape stacking: wizard over advisor unwinds one layer per keypress", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("a");
  await expect(page.getByTestId("advisor-panel")).toBeVisible();

  // P opens the wizard on top of the advisor
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();

  // first Escape closes ONLY the wizard — the advisor underneath survives
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("wizard-modal")).not.toBeVisible();
  await expect(page.getByTestId("advisor-panel")).toBeVisible();

  // second Escape closes the advisor — everything back as found
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("advisor-panel")).not.toBeVisible();
});
