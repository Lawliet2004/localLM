import { expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { connectSmoke, openSettingsSection } from './smoke-lib.mjs';

if (process.env.LOCALLM_NATIVE_SMOKE_ISOLATED !== '1') {
  console.error("Refusing to run: this smoke drives the running app's real app-data. Set LOCALLM_NATIVE_SMOKE_ISOLATED=1 only against an isolated debug instance.");
  process.exit(1);
}
let { browser, page } = await connectSmoke();
try {
    const invoke = command => page.evaluate(async command => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command); }, command);
  const original = (await invoke('bootstrap')).preferences;
  const items = await invoke('list_installed_runtimes');
  const item = items.find(item => item.complete);
  expect(item).toBeTruthy();
  await openSettingsSection(page, 'Models');
  const panel = page.getByRole('region', { name: 'CUDA runtime download' });
  await panel.getByText(/^Installed runtimes \(/).click();
  await panel.getByRole('button', { name: `Select ${item.id.slice(-8)}`, exact: true }).click();
  await expect(page.getByRole('textbox', { name: /^llama-server executable/ })).toHaveValue(item.path);
  expect((await invoke('bootstrap')).preferences).toEqual(original);
  writeFileSync('test-results/runtime-inventory-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), items, selected: item.id, preferencesPreserved: true }, null, 2));
  await page.reload();
} finally { await browser.close(); }
