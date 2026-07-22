// Multi-empire switcher (1.0): several named empires, each its own plan file
// beside the active one, switched from the DATA menu. The bridge mirrors the
// desktop Tauri commands; the web worker keeps the same shape over IndexedDB
// slots (covered by the web-smoke suite's build).
//
// The serial suite shares ONE bridge — every path here restores the original
// empire and deletes what it created, or every later spec would run against
// the wrong plan file.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}`);
  return res.json();
}
async function empires(request: APIRequestContext): Promise<{ active: string; names: string[] }> {
  const res = await request.get(`${API}/empires`);
  if (!res.ok()) throw new Error(`empires ${res.status()}`);
  return res.json();
}
async function empireOp(
  request: APIRequestContext,
  op: "create" | "switch" | "rename" | "delete",
  data: Record<string, string>,
): Promise<{ ok: boolean; status: number; body: any }> {
  const res = await request.post(`${API}/empire/${op}`, { data: JSON.stringify(data) });
  return { ok: res.ok(), status: res.status(), body: await res.json().catch(() => null) };
}

test("empires API: create/switch isolates plans; rename & delete are guarded", async ({ request }) => {
  const original = (await empires(request)).active;
  try {
    // create → the new EMPTY empire becomes active
    const created = await empireOp(request, "create", { name: "OUTPOST E2E" });
    expect(created.ok, JSON.stringify(created.body)).toBe(true);
    expect(created.body.active).toBe("OUTPOST E2E");
    expect(created.body.names).toContain(original);
    expect(Object.keys((await hydrate(request)).plan.factories)).toHaveLength(0);

    // duplicate create is refused
    expect((await empireOp(request, "create", { name: "OUTPOST E2E" })).status).toBe(422);

    // seed a factory in the new empire…
    await edit(request, [
      { type: "create_factory", name: "OUTPOST SEED", position: { x: 0, y: 0 }, region: "GRASS FIELDS" },
    ]);

    // …switch back: the original plan does NOT contain it
    expect((await empireOp(request, "switch", { name: original })).body.active).toBe(original);
    const names = Object.values<any>((await hydrate(request)).plan.factories).map((f) => f.name);
    expect(names).not.toContain("OUTPOST SEED");

    // …and switching forward again finds it persisted
    await empireOp(request, "switch", { name: "OUTPOST E2E" });
    const names2 = Object.values<any>((await hydrate(request)).plan.factories).map((f) => f.name);
    expect(names2).toContain("OUTPOST SEED");

    // renaming the ACTIVE empire reopens it under the new name, plan intact
    const renamed = await empireOp(request, "rename", { from: "OUTPOST E2E", to: "OUTPOST RENAMED" });
    expect(renamed.ok, JSON.stringify(renamed.body)).toBe(true);
    expect(renamed.body.active).toBe("OUTPOST RENAMED");
    expect(
      Object.values<any>((await hydrate(request)).plan.factories).map((f) => f.name),
    ).toContain("OUTPOST SEED");

    // deleting the active empire is refused; switch away first
    expect((await empireOp(request, "delete", { name: "OUTPOST RENAMED" })).status).toBe(422);
    await empireOp(request, "switch", { name: original });
    const deleted = await empireOp(request, "delete", { name: "OUTPOST RENAMED" });
    expect(deleted.ok).toBe(true);
    expect(deleted.body.names).not.toContain("OUTPOST RENAMED");

    // hostile names are refused, not sanitized into surprises
    expect((await empireOp(request, "create", { name: "../escape" })).status).toBe(422);
    expect((await empireOp(request, "create", { name: "   " })).status).toBe(422);
    expect((await empireOp(request, "switch", { name: "NO SUCH EMPIRE" })).status).toBe(422);
  } finally {
    // restore the shared bridge to the original empire; sweep test empires
    await empireOp(request, "switch", { name: original });
    for (const n of ["OUTPOST E2E", "OUTPOST RENAMED"]) {
      await empireOp(request, "delete", { name: n });
    }
    await resetView(request);
  }
});

test("empires UI: the DATA menu lists, creates and switches empires", async ({ page, request }) => {
  const original = (await empires(request)).active;
  try {
    await resetView(request);
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await expect(page.getByTestId("map-root")).toBeVisible();

    // open the DATA menu → the EMPIRES section lists the active empire
    await page.getByTestId("btn-data-menu").click();
    const section = page.getByTestId("empires-section");
    await expect(section).toBeVisible();
    await expect(section.getByTestId(`empire-row-${original}`)).toContainText(original);

    // create from the inline input → the app switches to the fresh empire
    await page.getByTestId("empire-new-name").fill("UI OUTPOST");
    await page.getByTestId("empire-create").click();
    await expect
      .poll(async () => (await empires(request)).active, { timeout: 10_000 })
      .toBe("UI OUTPOST");
    // the fresh empire hydrated in: no factories on the map
    await expect.poll(async () => Object.keys((await hydrate(request)).plan.factories).length).toBe(0);

    // switch back from the menu row
    await page.getByTestId("btn-data-menu").click();
    await page.getByTestId(`empire-switch-${original}`).click();
    await expect
      .poll(async () => (await empires(request)).active, { timeout: 10_000 })
      .toBe(original);
  } finally {
    await empireOp(request, "switch", { name: original });
    await empireOp(request, "delete", { name: "UI OUTPOST" });
    await resetView(request);
  }
});
