import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
const record = 'test-results/install-recovery-fixture.json';
if (process.argv.includes('--prepare')) {
  // Run only after the native app is closed, before launching the new build.
  const root = join(process.env.APPDATA, 'app.locallm.desktop');
  const model = join(root, 'models', `.locallm-download-${randomUUID().replaceAll('-', '')}.part`);
  const runtime = join(root, 'runtimes', `.locallm-runtime-${randomUUID().replaceAll('-', '')}`);
  mkdirSync(runtime);
  mkdirSync(join(runtime, 'bundle'));
  writeFileSync(join(runtime, 'bundle', 'llama-server.exe'), 'partial fixture', { flag: 'wx' });
  writeFileSync(model, 'partial fixture', { flag: 'wx' });
  writeFileSync(record, JSON.stringify({ model, runtime }));
} else {
  const fixture = JSON.parse(readFileSync(record, 'utf8'));
  const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
  try {
    let page;
    await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
    const inventory = await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke('list_installed_runtimes'); });
    expect(existsSync(fixture.model)).toBe(false);
    expect(existsSync(fixture.runtime)).toBe(false);
    expect(inventory.some(item => item.complete)).toBe(true);
    writeFileSync('test-results/install-recovery-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), partialModelRemoved: true, partialRuntimeRemoved: true, completedRuntimeRetained: true }, null, 2));
  } finally { await browser.close(); }
}
