// Headless browser proof for the offline viewer, using invented data only.
// Run with Playwright installed outside the repository:
//   NODE_PATH="$(npm root -g)" node contrib/graph/test_graph.js
// Set CHROMIUM to a browser binary to use one Playwright did not download.
const assert = require('node:assert/strict');
const path = require('node:path');
const { chromium } = require('playwright');

async function graph_tabs_date_range_and_all_history_use_synthetic_fixture() {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM,
    headless: true,
    args: ['--disable-gpu'],
  });
  try {
    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    await page.goto('file://' + path.join(__dirname, 'graph.html'));
    await page.setInputFiles('#file', path.join(__dirname, 'testdata', 'projects.json'));
    await page.waitForSelector('#table tbody tr');
    assert.match(await page.locator('#summary').innerText(), /^79 claims in view/);

    await page.locator('#tab-notes').click();
    assert.equal(await page.locator('#tab-notes').getAttribute('aria-selected'), 'true');
    assert.equal(await page.locator('#notes').isVisible(), true);
    assert.match(await page.locator('#note-list').innerText(), /invented office floors/);

    await page.locator('#tab-graph').click();
    assert.equal(await page.locator('#tab-graph').getAttribute('aria-selected'), 'true');
    assert.equal(await page.locator('#graph-panel').isVisible(), true);
    await page.waitForSelector('#graph g.center');

    await page.locator('#from').fill('2026-01-02');
    await page.locator('#to').fill('2026-01-02');
    await page.locator('#to').dispatchEvent('change');
    assert.match(await page.locator('#summary').innerText(), /^5 claims in view/);

    await page.locator('#from').fill('');
    await page.locator('#from').dispatchEvent('change');
    await page.locator('#to').fill('');
    await page.locator('#to').dispatchEvent('change');
    await page.locator('#tab-entities').click();
    await page.locator('#group').selectOption('is Task');
    assert.equal(await page.locator('#table .old').count(), 0);

    await page.locator('#status').selectOption('any');
    assert.match(await page.locator('#summary').innerText(), /^80 claims in view/);
    assert.equal(await page.locator('#table .old').count(), 1);
    assert.match(await page.locator('#table .old').getAttribute('title'), /invalidated: reassigned/);
  } finally {
    await browser.close();
  }
}

graph_tabs_date_range_and_all_history_use_synthetic_fixture().catch(error => {
  console.error(error);
  process.exit(1);
});
