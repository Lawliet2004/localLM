import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
try {
  let page;
  await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'Model files' }).click();
  const panel = page.getByRole('region', { name: 'Recommended model download' });
  await expect(panel.getByText(/GiB download/)).toBeVisible();
  await expect(panel.getByRole('alert')).toHaveCount(0);
  const metadata = await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke('model_download_info'); });
  expect(metadata.bytes).toBe(2070227904);
  expect(metadata.requiredBytes).toBe(metadata.bytes + 268435456);
  expect(metadata.availableBytes).toBeGreaterThan(0);
  expect(metadata.destination).toContain('MiniCPM5-2B.Q6_K.gguf');
  await page.screenshot({ path: 'test-results/model-download-info.png' });
  writeFileSync('test-results/model-download-info.json', JSON.stringify({ ...metadata, downloadStarted: false }, null, 2));
} finally { await browser.close(); }
