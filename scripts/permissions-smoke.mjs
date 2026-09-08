import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
const folder = resolve('.local', `permissions-smoke-${randomUUID()}`);
mkdirSync(folder, { recursive: true });
writeFileSync(join(folder, 'read.txt'), 'permission read verified');
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(page => page.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
async function invoke(command, args = {}) {
  return page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
}
const originalWorkspace = await invoke('get_workspace');
const originalPreferences = (await invoke('bootstrap')).preferences;
const report = { testedAt: new Date().toISOString(), scenarios: [] };
try {
  await invoke('save_preferences', { preferences: { ...originalPreferences, maxTokens: 2048, temperature: 0 } });
  await invoke('set_workspace', { path: folder });
  await page.reload();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  }
  const scenarios = [
    { mode: 'ask', tool: 'read_file', prompt: 'Use read_file to read read.txt exactly once. If denied, stop immediately and say denied.', ask: true },
    { mode: 'autoApprove', tool: 'read_file', prompt: 'Use read_file with path read.txt exactly once. Do not list files first. Repeat the file contents.', ask: false },
    { mode: 'autoApprove', tool: 'create_file', prompt: 'Use create_file to create denied.txt with content exactly "denied write". If denied, stop.', ask: true },
    { mode: 'fullAccess', tool: 'create_file', prompt: 'Use create_file to create allowed.txt with content exactly "full access verified". Do not add punctuation.', ask: false },
    { mode: 'fullAccess', tool: 'run_code', prompt: 'Use run_code with language python and code print(6 * 7). Execute it once and report stdout.', ask: false },
  ];
  for (const scenario of scenarios) {
    await page.getByRole('button', { name: /New conversation/ }).click();
    const mode = page.getByRole('combobox', { name: 'Permission mode' });
    await expect(mode).toHaveValue('ask');
    await mode.selectOption(scenario.mode);
    const picker = page.locator('.tool-picker');
    if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
    await page.getByRole('checkbox', { name: scenario.tool === 'run_code' ? 'Local code' : 'Workspace files', exact: true }).check();
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill(scenario.prompt);
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    await expect(mode).toBeDisabled();
    if (scenario.ask) {
      const dialog = page.getByRole('dialog');
      await expect(dialog).toBeVisible({ timeout: 180000 });
      await expect(dialog.getByText(scenario.tool, { exact: true })).toBeVisible();
      await dialog.getByRole('button', { name: 'Deny', exact: true }).click();
    } else {
      await expect.poll(async () => {
        if (await page.getByRole('dialog').count()) throw new Error('Unexpected approval prompt in automatic mode.');
        return await page.getByRole('button', { name: 'Stop response' }).count();
      }, { timeout: 180000 }).toBe(0);
    }
    await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
    await expect(page.getByRole('alert')).toHaveCount(0);
    const state = await invoke('bootstrap');
    const id = state.conversations[0].id;
    const messages = await invoke('get_messages', { id });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    expect(audits).toHaveLength(1);
    expect(audits[0].request.name).toBe(scenario.tool);
    expect(audits[0].request.accessMode).toBe(scenario.mode);
    expect(audits[0].request.decision).toBe(scenario.ask ? 'denied' : 'allowed');
    if (!scenario.ask) expect(audits[0].result.isError).not.toBe(true);
    if (scenario.tool === 'run_code') expect(audits[0].result.stdout.trim()).toBe('42');
    expect((await invoke('get_conversation_tools', { id })).accessMode).toBe(scenario.mode);
    report.scenarios.push({ mode: scenario.mode, tool: scenario.tool, prompted: scenario.ask, authorization: audits[0].request.authorization });
  }
  expect(existsSync(join(folder, 'denied.txt'))).toBe(false);
  expect(readFileSync(join(folder, 'allowed.txt'), 'utf8')).toBe('full access verified');
  const state = await invoke('bootstrap');
  await page.reload();
  await page.getByRole('button', { name: state.conversations[0].title, exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Permission mode' })).toHaveValue('fullAccess');
  await page.screenshot({ path: 'test-results/native-permissions.png' });
  await page.getByRole('button', { name: /New conversation/ }).click();
  await expect(page.getByRole('combobox', { name: 'Permission mode' })).toHaveValue('ask');
  writeFileSync('test-results/permissions-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  await invoke('cancel_generation');
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 20000 });
  await invoke('set_workspace', { path: originalWorkspace.path });
  await invoke('save_preferences', { preferences: originalPreferences });
  await browser.close();
}
