import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
try {
  let page;
  await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'Diagnostics' }).click();
  await expect(page.getByLabel('Runtime log output')).toBeVisible();
  const text = await page.getByLabel('Runtime log output').textContent();
  expect(text.length).toBeGreaterThan(0);
  expect(text.length).toBeLessThanOrEqual(65536);
  await page.getByRole('button', { name: 'Refresh log' }).click();
  await expect(page.getByRole('button', { name: 'Refresh log' })).toBeEnabled();
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.screenshot({ path: 'test-results/runtime-diagnostics.png' });
  writeFileSync('test-results/runtime-diagnostics.json', JSON.stringify({ testedAt: new Date().toISOString(), displayedCharacters: text.length, refresh: true }));
} finally { await browser.close(); }
