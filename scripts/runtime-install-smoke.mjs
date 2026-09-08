import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(page => page.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview required.');
const invoke = (command, args = {}) => page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
const original = await invoke('bootstrap');
const reuse = process.argv.includes('--reuse-installed');
let changed = false;
try {
  const before = await invoke('runtime_install_status');
  if (before.busy) throw new Error('An installation is already running; do not duplicate it.');
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'Model files', exact: true }).click();
  const panel = page.getByRole('region', { name: 'CUDA runtime download' });
  if (reuse && before.phase !== 'ready') throw new Error('No completed installation is available to resume.');
  if (!reuse) await expect(panel.getByRole('button', { name: 'Install CUDA runtime' })).toBeEnabled();
  const started = Date.now();
  if (!reuse) await panel.getByRole('button', { name: 'Install CUDA runtime' }).click();
  let installed;
  let lastLog = 0;
  await expect.poll(async () => {
    installed = await invoke('runtime_install_status');
    if (Date.now() - lastLog > 30000) { console.log(JSON.stringify(installed)); lastLog = Date.now(); }
    return !installed.busy && ['ready', 'failed', 'interrupted'].includes(installed.phase);
  }, { timeout: 7500000, intervals: [1000] }).toBe(true);
  if (installed.phase !== 'ready') throw new Error(installed.error || installed.phase);
  await panel.getByRole('button', { name: 'Use installed runtime' }).click();
  await expect(page.getByRole('textbox', { name: /^llama-server executable/ })).toHaveValue(installed.path);
  expect((await invoke('bootstrap')).preferences).toEqual(original.preferences);
  if (original.runtime.phase === 'ready') await invoke('unload_model');
  await invoke('save_preferences', { preferences: { ...original.preferences, runtimePath: installed.path } });
  changed = true;
  const runtime = await invoke('load_model');
  expect(runtime.phase).toBe('ready');
  expect(runtime.gpuOffload.layers).toBeGreaterThan(0);
  const report = { testedAt: new Date().toISOString(), installed, runtime, reusedCompletedInstallation: reuse, elapsedMs: Date.now() - started };
  writeFileSync('test-results/runtime-install-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  if (changed) {
    await invoke('unload_model');
    await invoke('save_preferences', { preferences: original.preferences });
    if (original.runtime.phase === 'ready') await invoke('load_model');
  }
  await page.reload();
  await browser.close();
}
