// A factory made by the goal wizard (P → PLAN SUPPLY CHAIN) must come out
// ALREADY laid out by the layered auto-layout, not on the raw emit-order grid —
// the wizard appends a TidyLayout as the last command of its create item. Proof:
// an explicit re-tidy right after accept moves NOTHING (idempotent → it was
// already tidy at creation). Self-contained: it creates one factory and deletes
// it, so the shared serial-suite plan is unchanged.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";
const hydrate = async (r: APIRequestContext): Promise<any> => (await r.get(`${API}/hydrate`)).json();
const edit = (r: APIRequestContext, cmds: unknown[]) => r.post(`${API}/edit`, { data: JSON.stringify(cmds) });

test("a wizard-made factory is auto-tidied on accept (no manual TIDY needed)", async ({ page, request }) => {
  await resetView(request);
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();

  const before = new Set(Object.keys((await hydrate(request)).plan.factories));

  // Goal wizard → a multi-stage material factory (iron rod: ore → ingot → rod).
  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await page.getByTestId("wizard-item").fill("iron rod");
  await page.getByTestId("wizard-item-option").first().click();
  await page.fill('[data-testid="wizard-rate"]', "30");
  await page.click('[data-testid="wizard-solve"]');
  await expect(page.getByTestId("proposal-review")).toBeVisible({ timeout: 20_000 });
  await page.getByTestId("btn-accept-proposal").click();
  await expect(page.getByTestId("proposal-review")).toHaveCount(0);

  const h = await hydrate(request);
  const fid = Object.keys(h.plan.factories).find((id) => !before.has(id));
  expect(fid, "the wizard created a new factory").toBeTruthy();

  try {
    const posOf = (hy: any): string => {
      const f = hy.plan.factories[fid!];
      const nodes = [
        ...f.groups.map((id: string) => hy.plan.groups[id]?.graphPos),
        ...f.ports.map((id: string) => hy.plan.ports[id]?.graphPos),
        ...Object.values<any>(hy.plan.junctions)
          .filter((j) => j.factory === fid)
          .map((j) => j.graphPos),
      ];
      return JSON.stringify(nodes);
    };
    // A wizard factory with several stages must be multi-node to make the check
    // meaningful (a single node is trivially "tidy").
    expect(h.plan.factories[fid!].groups.length, "the plan has real stages to lay out").toBeGreaterThan(1);

    const posBefore = posOf(h);
    // An explicit re-tidy is a no-op iff the factory was already laid out at
    // creation — which is exactly what the wizard's appended TidyLayout does.
    await edit(request, [{ type: "tidy_layout", factory: fid }]);
    const posAfter = posOf(await hydrate(request));
    expect(posAfter, "a wizard-made factory is already tidy — a re-tidy moves nothing").toBe(posBefore);
  } finally {
    await edit(request, [{ type: "delete_factory", id: fid }]).catch(() => {});
  }
});
