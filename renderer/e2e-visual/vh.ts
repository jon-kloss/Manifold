// Shared helpers for the visual map suite: dev-bridge API access, deterministic
// boot, pin geometry, and the screenshot/result recorder the HTML report reads.

import fs from "node:fs";
import path from "node:path";
import { expect, type APIRequestContext, type Page, type TestInfo } from "@playwright/test";

export const API = "http://localhost:8791/api";
// tests run with cwd = renderer/ (the config dir)
export const OUT = path.resolve("e2e-visual/out");
const SHOTS = path.join(OUT, "shots");
const RESULTS = path.join(OUT, "results.jsonl");

function append(entry: Record<string, unknown>): void {
  fs.mkdirSync(SHOTS, { recursive: true });
  fs.appendFileSync(RESULTS, JSON.stringify({ ts: Date.now(), ...entry }) + "\n");
}

/** Screenshot the full page into out/shots and record it for the report. */
export async function shot(page: Page, testInfo: TestInfo, slug: string, caption: string): Promise<void> {
  // order-stable name: test index + per-test sequence
  const seq = ((testInfo as unknown as { __shotSeq?: number }).__shotSeq ?? 0) + 1;
  (testInfo as unknown as { __shotSeq?: number }).__shotSeq = seq;
  const test = testInfo.title;
  const nn = test.slice(0, 2); // tests are titled "01 …", "02 …"
  const file = `${nn}-${String(seq).padStart(2, "0")}-${slug}.png`;
  await page.screenshot({ path: path.join(SHOTS, file) });
  append({ kind: "shot", test, file, caption });
}

/** Record the test verdict — wire this from an afterEach hook. */
export function recordVerdict(testInfo: TestInfo): void {
  append({
    kind: "verdict",
    test: testInfo.title,
    status: testInfo.status ?? "unknown",
    error: testInfo.error?.message?.split("\n").slice(0, 12).join("\n"),
    durationMs: testInfo.duration,
  });
}

// ---- dev-bridge API ----

export async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
export async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}: ${await res.text()}`);
  return res.json();
}

export async function newEmpire(request: APIRequestContext): Promise<void> {
  const res = await request.post(`${API}/new_empire`, { data: "{}" });
  if (!res.ok()) throw new Error(`new_empire ${res.status()}: ${await res.text()}`);
}

export async function resetView(request: APIRequestContext): Promise<void> {
  const res = await request.post(`${API}/view`, { data: JSON.stringify({ resumeSeen: true }) });
  if (!res.ok()) throw new Error(`resetView ${res.status()}: ${await res.text()}`);
}

/** Deterministic boot to the world map (API seeds never stream to an open
 *  client, so call AFTER seeding). */
export async function bootMap(page: Page, request: APIRequestContext): Promise<void> {
  await resetView(request);
  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
  await expect(page.getByTestId("map-root")).toBeVisible();
}

// ---- map geometry ----

/** Center of a factory's map pin; polls because a one-shot boundingBox races
 *  map init / zoom animation. */
export async function pinCenter(page: Page, name: string): Promise<{ x: number; y: number }> {
  const loc = page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`);
  let box = null;
  for (let i = 0; i < 25 && !box; i++) {
    box = await loc.boundingBox().catch(() => null);
    if (!box) await page.waitForTimeout(200);
  }
  if (!box) throw new Error(`pin not found: ${name}`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** Right-drag between two points — the route-drawing gesture. */
export async function rightDrag(
  page: Page,
  from: { x: number; y: number },
  to: { x: number; y: number },
): Promise<void> {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 5 });
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up({ button: "right" });
}

// ---- API seeding shorthand ----

export async function mkFactory(
  request: APIRequestContext,
  name: string,
  x: number,
  y: number,
): Promise<string> {
  return (
    await edit(request, [{ type: "create_factory", name, position: { x, y }, region: "GRASS FIELDS" }])
  ).created[0];
}

export async function mkPort(
  request: APIRequestContext,
  factory: string,
  direction: "in" | "out",
  item: string,
  ceiling: number | null,
  x: number,
): Promise<string> {
  return (
    await edit(request, [
      { type: "add_port", factory, direction, item, rate: 0, rateCeiling: ceiling, graphPos: { x, y: 100 } },
    ])
  ).created[0];
}

export async function mkGroup(
  request: APIRequestContext,
  factory: string,
  machine: string,
  recipe: string,
  x = 300,
): Promise<string> {
  return (
    await edit(request, [
      { type: "add_group", factory, machine, recipe, count: 1, clock: 1.0, graphPos: { x, y: 100 }, floor: 0 },
    ])
  ).created[0];
}

export const G = (id: string) => ({ kind: "group", id });
export const P = (id: string) => ({ kind: "port", id });

export async function belt(
  request: APIRequestContext,
  factory: string,
  from: unknown,
  to: unknown,
  item: string,
  tier = 3,
): Promise<void> {
  await edit(request, [{ type: "add_edge", factory, from, to, item, tier }]);
}
