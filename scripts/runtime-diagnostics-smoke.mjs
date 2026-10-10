import { expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { connectSmoke, openSettingsSection } from './smoke-lib.mjs';

if (process.env.LOCALLM_NATIVE_SMOKE_ISOLATED !== '1') {
  console.error("Refusing to run: this smoke drives the running app's real app-data. Set LOCALLM_NATIVE_SMOKE_ISOLATED=1 only against an isolated debug instance.");
  process.exit(1);
}
let { browser, page } = await connectSmoke();
try {
    await openSettingsSection(page, 'Models');
  await openSettingsSection(page, 'Diagnostics');
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
