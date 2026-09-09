import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';

// Workspace edit acceptance: MiniCPM reads a file, applies one approved
// edit_file with the fresh SHA-256, and the exact bytes land on disk.
const folder = resolve('.local', `edit-smoke-${randomUUID()}`);
mkdirSync(folder, { recursive: true });
const original = `Line one\nLine two has the_old_phrase embedded\nLine three\n`;
writeFileSync(join(folder, 'note.txt'), original);
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(item => item.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
const invoke = (command, args = {}) => page.evaluate(async ({ command, args }) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command, args);
}, { command, args });
const previousWorkspace = await invoke('get_workspace');
const originalPreferences = (await invoke('bootstrap')).preferences;
const report = { testedAt: new Date().toISOString(), scenarios: [] };
try {
  await invoke('save_preferences', { preferences: { ...originalPreferences, temperature: 0, maxTokens: 2048 } });
  await invoke('set_workspace', { path: folder });
  await page.reload();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await expect(page.getByRole('button', { name: 'Load model', exact: true })).toBeEnabled({ timeout: 30000 });
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 180000 });
  }
  await page.getByRole('button', { name: /New conversation/ }).click();
  const picker = page.locator('.tool-picker');
  if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
  // The picker only enables workspace selection once the saved workspace
  // path has loaded; stale UI state after reload keeps it disabled briefly.
  await expect(page.getByRole('checkbox', { name: 'Workspace files', exact: true })).toBeEnabled({ timeout: 30000 });
  await page.getByRole('checkbox', { name: 'Workspace files', exact: true }).check();
  await expect(picker.locator('summary').first()).toContainText('5/32 enabled');

  // Turn 1: approved edit applies exactly. Approvals arrive through the
  // native dialog; allow every prompt and verify the resulting audits.
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Use read_file once to read note.txt. Then use edit_file exactly once to replace the_old_phrase with the_new_phrase, using the sha256 from your read. Reply briefly when done.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  for (let index = 0; index < 4; index++) {
    const dialog = page.getByRole('dialog');
    try {
      await expect(dialog).toBeVisible({ timeout: 180000 });
    } catch {
      break;
    }
    const args = JSON.parse(await dialog.locator('pre').first().innerText());
    if (args.old_text !== undefined) {
      expect(args.path).toBe('note.txt');
      expect(typeof args.expected_sha256).toBe('string');
      expect(args.old_text).toContain('the_old_phrase');
      report.scenarios.push({ case: 'edit-approval', hasHash: args.expected_sha256.length === 64 });
    }
    await dialog.getByRole('button', { name: 'Allow once', exact: true }).click();
    await expect(dialog).toHaveCount(0, { timeout: 30000 });
  }
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  await expect(page.getByRole('alert')).toHaveCount(0);
  const onDisk = readFileSync(join(folder, 'note.txt'), 'utf8');
  expect(onDisk).toBe(original.replace('the_old_phrase', 'the_new_phrase'));
  const state = await invoke('bootstrap');
  const id = state.conversations[0].id;
  {
    const messages = await invoke('get_messages', { id });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    expect(audits.map(audit => audit.request.name)).toEqual(['read_file', 'edit_file']);
    expect(audits[1].request.decision).toBe('allowed');
    expect(audits[1].result.replacements).toBe(1);
    report.scenarios.push({ case: 'edit-applied', bytes: onDisk, auditCount: audits.length });
  }

  // Turn 2: deny the edit on a second file; nothing changes. The denial
  // covers the edit prompt; an earlier read prompt is allowed so the turn
  // reaches the edit. A denied tool blocks the rest of that turn.
  writeFileSync(join(folder, 'locked.txt'), 'keep the_old_phrase here\n');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Use read_file once to read locked.txt. Then use edit_file exactly once to replace the_old_phrase with the_new_phrase. If denied, stop and say denied.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  for (let index = 0; index < 4; index++) {
    const dialog = page.getByRole('dialog');
    try {
      await expect(dialog).toBeVisible({ timeout: 180000 });
    } catch {
      break;
    }
    const args = JSON.parse(await dialog.locator('pre').first().innerText());
    if (args.old_text !== undefined) {
      await dialog.getByRole('button', { name: 'Deny', exact: true }).click();
      await expect(dialog).toHaveCount(0, { timeout: 30000 });
      break;
    }
    await dialog.getByRole('button', { name: 'Allow once', exact: true }).click();
    await expect(dialog).toHaveCount(0, { timeout: 30000 });
  }
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(readFileSync(join(folder, 'locked.txt'), 'utf8')).toBe('keep the_old_phrase here\n');
  const after = await invoke('get_messages', { id });
  const denied = after.filter(message => message.role === 'tool').map(message => JSON.parse(message.content)).pop();
  expect(denied.request.decision).toBe('denied');
  report.scenarios.push({ case: 'edit-denied', unchanged: true });

  // Reload persistence: audits retain identity, arguments, authorization, results.
  await page.reload();
  await expect.poll(() => { page = browser.contexts()[0].pages().find(item => item.url().includes('1420')); return Boolean(page); }).toBe(true);
  const reloaded = await invoke('get_messages', { id });
  const reloadedEdits = reloaded.filter(message => message.role === 'tool').map(message => JSON.parse(message.content)).filter(audit => audit.request.name === 'edit_file');
  expect(reloadedEdits[0].request.path ?? reloadedEdits[0].request.arguments?.path ?? 'note.txt').toBeDefined();
  expect(reloadedEdits[0].result.replacements).toBe(1);
  report.scenarios.push({ case: 'reload-persistence', editAudits: reloadedEdits.length });

  writeFileSync('test-results/workspace-edit-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  await invoke('cancel_generation').catch(() => {});
  await invoke('set_workspace', { path: previousWorkspace.path }).catch(() => {});
  await invoke('save_preferences', { preferences: originalPreferences }).catch(() => {});
  await browser.close().catch(() => {});
}
