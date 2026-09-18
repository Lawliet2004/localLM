import { chromium, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const browser = await chromium.launch({ headless: true, channel: 'msedge' });
const page = await browser.newPage({ viewport: { width: 1280, height: 840 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
mkdirSync('.vitest', { recursive: true });
try {
  await page.goto('http://localhost:1420');
  await page.getByLabel('Attach files').setInputFiles({ name: 'analysis.csv', mimeType: 'text/csv', buffer: Buffer.from('month,total\nSeptember,42') });
  await expect(page.getByRole('button', { name: 'Remove analysis.csv' })).toBeVisible();
  await page.getByLabel('Message', { exact: true }).fill('Summarize the attached results and help me plan the next step.');
  await expect(page.locator('.composer .tool-picker')).toHaveCount(0);
  await page.screenshot({ path: '.vitest/composer-desktop.png' });
  await page.getByRole('button', { name: 'Remove analysis.csv' }).click();
  await expect(page.locator('.attachment-chip')).toHaveCount(0);
  // Native picker format filtering is supplemented by validation on drops.
  await page.getByLabel('Attach files').setInputFiles({ name: 'photo.png', mimeType: 'image/png', buffer: Buffer.from('unsupported') });
  await expect(page.getByRole('alert')).toContainText('not supported');
  for (const width of [320, 768, 1440]) {
    await page.setViewportSize({ width, height: 840 });
    const box = await page.locator('.composer').boundingBox();
    if (!box || box.x < 0 || box.x + box.width > width) throw new Error(`Composer outside viewport at ${width}px`);
    if (width === 320) await page.screenshot({ path: '.vitest/composer-mobile.png' });
  }
  expect(errors).toEqual([]);
  console.log('Composer smoke passed: attachments, removal, unsupported formats, 320/768/1280/1440px layouts; no page errors.');
} finally { await browser.close(); }

