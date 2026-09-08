import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
try {
  let page;
  await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
  const invoke = command => page.evaluate(async command => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command); }, command);
  const original = (await invoke('bootstrap')).preferences;
  const items = await invoke('list_installed_runtimes');
  const item = items.find(item => item.complete);
  expect(item).toBeTruthy();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  const panel = page.getByRole('region', { name: 'CUDA runtime download' });
  await panel.getByText(/^Installed runtimes \(/).click();
  await panel.getByRole('button', { name: `Select ${item.id.slice(-8)}`, exact: true }).click();
  await expect(page.getByRole('textbox', { name: /^llama-server executable/ })).toHaveValue(item.path);
  expect((await invoke('bootstrap')).preferences).toEqual(original);
  writeFileSync('test-results/runtime-inventory-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), items, selected: item.id, preferencesPreserved: true }, null, 2));
  await page.reload();
} finally { await browser.close(); }
