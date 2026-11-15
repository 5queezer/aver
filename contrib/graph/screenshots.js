// Regenerate the README screenshots from the invented projects fixture.
// Needs Playwright with Chromium, installed outside this repository:
//   NODE_PATH="$(npm root -g)" node contrib/graph/screenshots.js
// Set CHROMIUM to a browser binary to use one Playwright did not download.
const { chromium } = require('playwright');
const path = require('path');
const root = path.join(__dirname, '..', '..');
(async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROMIUM, args: ['--disable-gpu'] });
  const shoot = async (viewport, run) => {
    const page = await browser.newPage({ viewport });
    await page.goto('file://' + path.join(__dirname, 'graph.html'));
    await page.setInputFiles('#file', path.join(__dirname, 'testdata', 'projects.json'));
    await page.waitForSelector('#table tbody tr');
    await run(page);
    await page.close();
  };
  await shoot({ width: 1500, height: 1000 }, async page => {
    const box = await page.locator('section').boundingBox();
    const table = await page.locator('#entities .scroll').boundingBox();
    await page.screenshot({ path: path.join(root, 'doc/images/graph-entities.png'),
      clip: { x: 0, y: 0, width: box.x + box.width, height: table.y + table.height + 16 } });
  });
  await shoot({ width: 1100, height: 900 }, async page => {
    await page.locator('#table tbody td:first-child', { hasText: /^task:5$/ }).click();
    await page.waitForSelector('#graph g.center');
    await page.screenshot({ path: path.join(root, 'doc/images/graph-focus.png') });
  });
  await browser.close();
})().catch(error => { console.error(error); process.exit(1); });
