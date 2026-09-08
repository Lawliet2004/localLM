import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
const folder = resolve('.local', `history-smoke-${randomUUID()}`);
mkdirSync(folder, { recursive: true });
const marker = `NIMBUS-${Math.floor(100000 + Math.random() * 900000)}`;
writeFileSync(join(folder, 'code.txt'), `Verification code: ${marker}\n`);
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(page => page.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
async function invoke(command, args = {}) {
  return page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
}
const previousWorkspace = await invoke('get_workspace');
try {
  await invoke('set_workspace', { path: folder });
  await page.reload();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  }
  await page.getByRole('button', { name: 'New conversation', exact: false }).click();
  const picker = page.locator('.tool-picker');
  if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
  await page.getByRole('checkbox', { name: 'Workspace files', exact: true }).check();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('History check ' + randomUUID().slice(0, 8) + '. Use read_file once to read code.txt. After reading it, reply only "Done." Do not repeat the file contents or the verification code in your response.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toBeVisible({ timeout: 180000 });
  await expect(dialog.getByText('read_file', { exact: true })).toBeVisible();
  expect(JSON.parse(await dialog.locator('pre').innerText()).path).toBe('code.txt');
  await dialog.getByRole('button', { name: 'Allow once', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  await expect(page.getByRole('alert')).toHaveCount(0);
  const firstAnswer = await page.locator('.message-assistant .markdown').innerText();
  expect(firstAnswer).not.toContain(marker);
  const state = await invoke('bootstrap');
  const id = state.conversations[0].id;
  await page.getByRole('checkbox', { name: 'Workspace files', exact: true }).click();
  await expect(picker.locator('summary').first()).toHaveText('Tools · Off');
  // Force restoration from SQLite rather than relying on any live turn state.
  await page.reload();
  await page.getByRole('button', { name: state.conversations[0].title, exact: true }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('What was the complete verification code in the file you already read? Reply with that exact code. Use the earlier tool result; no tools are enabled now.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  await expect(page.getByRole('alert')).toHaveCount(0);
  const answer = await page.locator('.message-assistant .markdown').last().innerText();
  expect(answer).toContain(marker);
  const messages = await invoke('get_messages', { id });
  expect(messages.filter(message => message.role === 'tool')).toHaveLength(1);
  const report = { testedAt: new Date().toISOString(), marker, firstAnswer, followupAnswer: answer, toolCalls: 1, reloadedBeforeFollowup: true };
  writeFileSync('test-results/history-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  await invoke('cancel_generation');
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 20000 });
  await invoke('set_workspace', { path: previousWorkspace.path });
  await browser.close();
}

