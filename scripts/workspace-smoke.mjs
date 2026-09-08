import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
const folder = resolve('.local', `workspace-smoke-${randomUUID()}`);
mkdirSync(folder, { recursive: true });
const marker = `LOCAL-${randomUUID()}`;
writeFileSync(join(folder, 'fixture.txt'), `The verification code is ${marker}.\n`);
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(value => value.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
try {
  await page.evaluate(async path => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); await invoke('set_workspace', { path }); }, folder);
  await page.reload();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  }
  const results = [];
  for (const mode of ['read', 'deny-write', 'write']) {
    await page.getByRole('button', { name: 'New conversation', exact: false }).click();
    const picker=page.locator('.tool-picker');
    if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
    await page.getByRole('checkbox', { name: 'Workspace files', exact: true }).check();
    const prompt = mode === 'read' ? 'Use read_file to read fixture.txt in the workspace. Quote the ENTIRE verification code verbatim, including its prefix. Do not shorten or reformat it.' : 'Use create_file to create output.txt in the workspace. Its entire content must be the JSON string value "workspace write verified" (without the quotes, with no added punctuation). If permission is denied, stop.';
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill(prompt);
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const dialog=page.getByRole('dialog');
    await expect(dialog).toBeVisible({ timeout: 180000 });
    await expect(dialog.getByText(mode === 'read' ? 'read_file' : 'create_file', { exact: true })).toBeVisible();
    const args=JSON.parse(await dialog.locator('pre').innerText());
    expect(args.path).toBe(mode === 'read' ? 'fixture.txt' : 'output.txt');
    if (mode !== 'read') expect(args.content).toBe('workspace write verified');
    await dialog.getByRole('button', { name: mode === 'deny-write' ? 'Deny' : 'Allow once', exact: true }).click();
    await expect(dialog).toHaveCount(0, { timeout: 15000 });
    await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
    await expect(page.getByRole('alert')).toHaveCount(0);
    const answer=await page.locator('.message-assistant .markdown').innerText();
    if (mode === 'read') expect(answer).toContain(marker);
    if (mode === 'deny-write') expect(existsSync(join(folder, 'output.txt'))).toBe(false);
    if (mode === 'write') expect(readFileSync(join(folder, 'output.txt'), 'utf8').trim()).toBe('workspace write verified');
    results.push({ mode, args, answer });
  }
  mkdirSync('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/native-workspace.png' });
  const report={ testedAt:new Date().toISOString(),folder,results };
  writeFileSync('test-results/workspace-smoke.json', JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
} finally {
  await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); await invoke('cancel_generation'); }).catch(() => {});
  await browser.close();
}
