import { chromium, expect } from '@playwright/test';
import { linkSync, existsSync, statSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
const invoke = (command) => page.evaluate(async command => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command); }, command);
try {
  const metadata = await invoke('model_download_info');
  const expected = join(process.env.APPDATA, 'app.locallm.desktop', 'models', 'MiniCPM5-2B.Q6_K.gguf');
  expect(resolve(metadata.destination)).toBe(resolve(expected));
  expect((await invoke('model_install_status')).busy).toBe(false);
  const original = (await invoke('bootstrap')).preferences;
  if (!existsSync(metadata.destination)) {
    const source = resolve('.local/models/MiniCPM5-2B.Q6_K.gguf');
    expect(statSync(source).size).toBe(metadata.bytes);
    // Retain this application-managed hard link as the installed model; no extra weight storage.
    linkSync(source, metadata.destination);
  }
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'Model files' }).click();
  const panel = page.getByRole('region', { name: 'Recommended model download' });
  await panel.getByRole('button', { name: 'Verify managed model' }).click();
  await expect.poll(async () => (await invoke('model_install_status')).phase, { timeout: 180000 }).toBe('ready');
  await expect(panel.getByRole('button', { name: 'Use verified model' })).toBeEnabled();
  expect((await invoke('bootstrap')).preferences).toEqual(original);
  await panel.getByRole('button', { name: 'Use verified model' }).click();
  await expect(page.getByLabel('GGUF model file', { exact: true })).toHaveValue(metadata.destination);
  expect((await invoke('bootstrap')).preferences).toEqual(original);
  writeFileSync('test-results/model-install-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), status: await invoke('model_install_status'), preferencePreserved: true, downloadedBytes: 0 }, null, 2));
  await page.reload();
} finally { await browser.close(); }
