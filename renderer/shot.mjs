// Screenshot the app surfaces + the reference mocks (2a, 4a) for comparison.
import { chromium } from "@playwright/test";

const out = process.argv[2] ?? "shots";
const browser = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium" });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
page.on("pageerror", (e) => console.log("PAGE ERR:", e.message));

await page.goto("http://localhost:5173");
await page.waitForSelector('[data-testid="graph-root"], [data-testid="map-root"]', { timeout: 15000 });
await page.waitForTimeout(1200);

// map first
if (await page.getByTestId("graph-root").isVisible().catch(() => false)) {
  await page.getByTestId("btn-world").click();
  await page.waitForTimeout(800);
}
await page.keyboard.press("f");
await page.waitForTimeout(700);
await page.screenshot({ path: `${out}/app-2a-map.png` });

// factory summary drawer → open factory
await page.locator(".pin-chip", { hasText: "MODULAR WORKS" }).click();
await page.waitForTimeout(500);
await page.screenshot({ path: `${out}/app-2b-drawer.png` });
await page.getByTestId("btn-open-factory").click();
await page.waitForTimeout(900);
await page.keyboard.press("f");
await page.waitForTimeout(600);
await page.getByTestId("port-out-Desc_ModularFrame_C").click();
await page.waitForTimeout(500);
await page.screenshot({ path: `${out}/app-4a-graph.png` });

// mocks
const mock = "file:///home/user/Conveyancer/design-reference/FICSIT Planner - Foundations.dc.html";
await page.goto(encodeURI(mock));
await page.waitForTimeout(2000);
for (const id of ["2a", "4a"]) {
  const el = page.locator(`[id="${id}"]`).first();
  try {
    await el.scrollIntoViewIfNeeded();
    await page.waitForTimeout(500);
    await el.screenshot({ path: `${out}/mock-${id}.png`, timeout: 10000 });
  } catch (e) {
    console.log(`mock ${id} failed:`, e.message.split("\n")[0]);
  }
}
console.log("done");
await browser.close();
