// Drives the demo page in headless Chromium: the engine loads, a drawing appears, a drag
// re-routes the edges, and the other examples render without errors.
//
//   pnpm --filter site build && pnpm --filter site serve --port 3011 &
//   SITE_URL=http://localhost:3011/graphlay node tests/smoke.mjs [screenshot-dir]
import { chromium } from "playwright";

const base = process.env.SITE_URL ?? "http://localhost:3000/graphlay";
const shots = process.argv[2];
// PW_CHROMIUM points at a browser binary when Playwright's own download is not available.
const browser = await chromium.launch({ executablePath: process.env.PW_CHROMIUM || undefined });
const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
const errors = [];
const report = (text) => {
  errors.push(text);
  console.log(text);
};
page.on("pageerror", (e) => report(`pageerror: ${e.message}`));
page.on("console", (m) => {
  if (m.type() === "error") report(`console: ${m.text()}`);
});

await page.goto(`${base}/demo`, { waitUntil: "networkidle" });
await page.waitForSelector("svg g[data-node]", { timeout: 60000 });
const nodes = await page.locator("svg g[data-node]").count();
const edges = await page.locator("svg path[marker-end]").count();
console.log(`nodes ${nodes}, edges ${edges}`);
console.log("metrics:", (await page.locator("table tbody tr").allInnerTexts()).join(" | "));

const before = await page.locator("svg path[marker-end]").first().getAttribute("d");
const shape = page.locator("svg g[data-node='0'] rect, svg g[data-node='0'] ellipse").first();
const box = await shape.boundingBox();
await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
await page.mouse.down();
await page.mouse.move(box.x + box.width / 2 + 60, box.y + box.height / 2 + 80, { steps: 8 });
await page.mouse.move(box.x + box.width / 2 + 120, box.y + box.height / 2 + 160, { steps: 8 });
// While the button is down the node is under the cursor, whatever the canvas's aspect ratio.
const held = await shape.boundingBox();
const dx = held.x - box.x;
const dy = held.y - box.y;
console.log(`node moved by ${dx.toFixed(1)}, ${dy.toFixed(1)} screen px for a 120, 160 drag`);
if (Math.abs(dx - 120) > 2 || Math.abs(dy - 160) > 2) report("the dragged node did not stay under the cursor");
await page.mouse.up();
await page.waitForTimeout(300);
const after = await page.locator("svg path[marker-end]").first().getAttribute("d");
console.log("edges re-routed after the drag:", before !== after);
if (shots) await page.screenshot({ path: `${shots}/demo-after-drag.png` });

for (const example of ["Petri net", "Hub and spoke", "Nested subprocesses", "Wrapped chain", "Swimlanes", "Random graph"]) {
  const select = page.locator("select").first();
  if ((await select.count()) === 0) {
    report(`the page lost its controls before ${example}`);
    break;
  }
  await select.selectOption({ label: example });
  await page.waitForTimeout(400);
  const n = await page.locator("svg g[data-node]").count();
  console.log(`${example}: ${n} nodes`);
  if (shots) await page.screenshot({ path: `${shots}/demo-${example.toLowerCase().replace(/ /g, "-")}.png` });
}

console.log("errors:", errors.length ? `\n${errors.join("\n")}` : "none");
await browser.close();
if (errors.length) process.exit(1);
