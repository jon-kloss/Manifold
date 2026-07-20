// Desktop save-sync parity: on the desktop/dev-bridge build the "Sync from
// save" + "Auto-sync" controls are available (they used to be web-only), driven
// through the bridge's sync mirror (Tauri IPC isn't scriptable by Playwright).
// The bridge's /api/sync/pick returns the fixture wired via FICSIT_SYNC_SAVE.

import { fileURLToPath } from "node:url";
import { test, expect, type Page } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";
const SAVES = fileURLToPath(new URL("../../fixtures/saves", import.meta.url));

async function importSave(page: Page, file: string) {
  await page.getByTestId("btn-data-menu").click();
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByTestId("btn-import").click(),
  ]);
  await chooser.setFiles(`${SAVES}/${file}`);
  await expect(page.getByTestId("import-preview")).toBeVisible({ timeout: 120_000 });
  await page.getByTestId("btn-import-run").click();
  await expect(page.getByTestId("import-done")).toBeVisible({ timeout: 60_000 });
  await page.locator(".wizard-foot .btn-primary").click(); // DONE
}

test("desktop build exposes Sync-from-save + Auto-sync and re-reads the native save", async ({
  page,
  request,
}) => {
  test.setTimeout(300_000); // two .sav parses in a cold worker
  // Clean slate on the shared bridge, and wipe again on exit so the imported
  // save doesn't leak into later specs (this file sorts before phase4-import).
  await request.post(`${API}/new_empire`, { data: "{}" });
  await resetView(request);
  try {
    await runDesktopSync(page);
  } finally {
    await request.post(`${API}/new_empire`, { data: "{}" }).catch(() => {});
  }
});

async function runDesktopSync(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // The sync block renders on the DESKTOP build (was __WASM_BACKEND__-gated).
  await page.getByTestId("btn-data-menu").click();
  const syncBtn = page.getByTestId("btn-sync-save");
  await expect(syncBtn).toBeVisible();
  // With nothing imported yet it's disabled (nothing to reconcile against).
  await expect(syncBtn).toHaveAttribute("aria-disabled", "true");
  await page.keyboard.press("Escape");

  // Import the save → now there's a ◆ built layer to sync against.
  await importSave(page, "Dunarr-076.sav");

  // Sync from save: the bridge stands in for the native picker (FICSIT_SYNC_SAVE)
  // → reads the same save → reconciles. Re-reading the just-imported save is
  // in-sync, so this is deterministic; the point is the desktop path runs.
  await page.getByTestId("btn-data-menu").click();
  await expect(syncBtn).not.toHaveAttribute("aria-disabled", "true");
  await syncBtn.click(); // closes the menu, runs the sync

  // Re-open: the control now shows it re-read the native save (sync-meta stuck).
  await expect
    .poll(
      async () => {
        await page.getByTestId("btn-data-menu").click();
        const txt = await page.getByTestId("btn-sync-save").innerText().catch(() => "");
        await page.keyboard.press("Escape");
        return txt;
      },
      { timeout: 60_000, intervals: [1000] },
    )
    .toContain("Dunarr-076.sav");

  // Auto-sync is available on desktop (native FS supports the silent re-read).
  await page.getByTestId("btn-data-menu").click();
  const auto = page.getByTestId("btn-auto-sync");
  await expect(auto).not.toHaveAttribute("aria-disabled", "true");
  await auto.click(); // turn on → one immediate pull + interval chips appear
  await expect(page.getByTestId("autosync-intervals")).toBeVisible();
  await expect(page.getByTestId("autosync-10")).toBeVisible();
  await expect(auto).toHaveAttribute("aria-checked", "true");
}
