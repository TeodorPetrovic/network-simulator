/**
 * Manual browser integration check for flyout drag and drop.
 * Build frontend (`cd frontend && npm run build`), start the simulator using
 * the screenshot recipe, create/open a project, then run:
 *   node scripts/ui_dragdrop_check.mjs http://127.0.0.1:PORT/projects/PROJECT_ID
 * Requires the workstation Playwright installation at the path below.
 */
import assert from 'node:assert/strict';
import { chromium } from '/home/teodor/.cache/qa-tools/node_modules/playwright-core/index.mjs';

const url = process.argv[2];
assert.ok(url, 'Pass the project URL as the first argument');
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1500, height: 900 } });
  await page.goto(url);
  await page.getByRole('button', { name: 'Routers' }).click();
  const row = page.locator('.sim-device-template').first();
  const canvas = page.locator('.sim-canvas-surface');
  await row.waitFor();
  const before = await page.locator('.sim-node').count();
  await row.dragTo(canvas, { targetPosition: { x: 430, y: 320 } });
  await page.waitForFunction((count) => document.querySelectorAll('.sim-node').length > count, before);
  assert.ok(await page.locator('.sim-node').count() > before, 'Drop should add a canvas node');
  console.log('Flyout drag and drop added a node.');
} finally { await browser.close(); }
