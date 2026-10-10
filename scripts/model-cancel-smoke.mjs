import { expect } from '@playwright/test';
import { statSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { connectSmoke, openSettingsSection } from './smoke-lib.mjs';

if (process.env.LOCALLM_NATIVE_SMOKE_ISOLATED !== '1') {
  console.error("Refusing to run: this smoke drives the running app's real app-data. Set LOCALLM_NATIVE_SMOKE_ISOLATED=1 only against an isolated debug instance.");
  process.exit(1);
}
let { browser, page } = await connectSmoke();
try {
    const invoke = command => page.evaluate(async command => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command); }, command);
  const metadata = await invoke('model_download_info');
  expect(metadata.destinationExists).toBe(true);
  expect((await invoke('model_install_status')).busy).toBe(false);
  const before = statSync(metadata.destination);
  const preferences = (await invoke('bootstrap')).preferences;
  await openSettingsSection(page, 'Models');
  const panel = page.getByRole('region', { name: 'Recommended model download' });
  await panel.getByRole('button', { name: 'Verify managed model' }).click();
  await expect.poll(async () => (await invoke('model_install_status')).received).toBeGreaterThan(0);
  await panel.getByRole('button', { name: 'Pause download', exact: true }).click();
  await expect.poll(async () => (await invoke('model_install_status')).busy).toBe(false);
  const status = await invoke('model_install_status');
  expect(status.error).toContain('cancelled');
  expect(status.phase).toBe('cancelled');
  expect(statSync(metadata.destination).size).toBe(before.size);
  expect(statSync(metadata.destination).mtimeMs).toBe(before.mtimeMs);
  expect((await invoke('bootstrap')).preferences).toEqual(preferences);
  expect(readdirSync(dirname(metadata.destination)).filter(name => name.startsWith('.locallm-download-'))).toEqual([]);
  await expect(panel.getByRole('button', { name: 'Verify managed model' })).toBeEnabled();
  writeFileSync('test-results/model-cancel-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), status, fileSizeAndModificationTimePreserved: true, preferencesPreserved: true, noPartialFiles: true }, null, 2));
} finally { await browser.close(); }
