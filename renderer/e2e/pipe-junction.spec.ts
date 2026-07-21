// Pipe Junction — the single 4-way Pipeline Junction Cross that both merges and
// splits FLUID lines. This spec owns the UI-unique coverage: the + LOGISTIC
// catalog offers it and the placed node renders in pipe-blue. The fluid-only
// connect rule + the 4-port cap are exhaustively covered by the Rust session
// tests (pipe_junction_*), so they're not re-checked over the raw /edit surface.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function openGraph(page: any, name: string): Promise<void> {
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
  await expect(page.locator(".react-flow__pane")).toBeVisible();
  await page.waitForTimeout(300);
}
async function dismissOnboarding(page: any): Promise<void> {
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
}

test("the catalog offers a Pipeline Junction and it renders pipe-blue", async ({ page, request }) => {
  await resetView(request);
  const f = (
    await edit(request, [
      { type: "create_factory", name: "PIPE HALL", position: { x: -1000, y: 2100 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await openGraph(page, "PIPE HALL");

    // The + LOGISTIC catalog offers the fluid junction alongside the belt ones.
    await page.getByTestId("btn-logistic").click();
    const menu = page.getByTestId("logistic-menu");
    await expect(menu.getByRole("button", { name: "Pipeline Junction" })).toBeVisible();
    await menu.getByRole("button", { name: "Pipeline Junction" }).click();

    // It renders as a distinct pipe-junction node (its own testid kind), carrying
    // the `pipe` class.
    const node = page.locator('[data-testid^="junction-pipe_junction-"]');
    await expect(node).toBeVisible();
    await expect(node).toHaveClass(/\bpipe\b/);

    // Deselect (the just-placed node is selected → signal border) via Escape,
    // which the graph handler uses to clear node selection. Then the `pipe`
    // class actually paints the border pipe-blue (--bp-400 = #56A8FF =
    // rgb(86,168,255)) — the styling isn't just a class name. Selection's own
    // signal border correctly wins WHILE selected (see graph.css cascade order).
    await page.keyboard.press("Escape");
    await expect(node).not.toHaveClass(/\bselected\b/);
    await expect(node).toHaveCSS("border-color", "rgb(86, 168, 255)");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
