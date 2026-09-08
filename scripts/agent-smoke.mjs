import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(value => value.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
const report = { testedAt: new Date().toISOString(), scenarios: [] };
try {
  await page.getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search connectors' }).fill('deepwiki');
  const connector = page.locator('.catalog-item');
  if (await connector.getAttribute('open') === null) await connector.locator('summary').first().click();
  if (await connector.getByRole('button', { name: 'Connect', exact: true }).count()) {
    await connector.getByRole('button', { name: 'Connect', exact: true }).click();
    await expect(connector.getByRole('button', { name: 'Disconnect', exact: true })).toBeVisible({ timeout: 70000 });
  }
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  }
  for (const decision of ['allow', 'deny', 'cancel']) {
    await page.getByRole('button', { name: 'New conversation', exact: false }).click();
    const picker = page.locator('.tool-picker');
    if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
    const group = picker.locator('.connector-tool-group').filter({ hasText: 'deepwiki' });
    if (await group.getAttribute('open') === null) await group.locator('summary').click();
    await group.getByRole('checkbox', { name: 'read_wiki_structure', exact: true }).check();
    await expect(picker.locator('summary').first()).toContainText('1/32 enabled');
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Use the read_wiki_structure tool for the public repository tauri-apps/tauri. Call it exactly once, then briefly name one section from its result. If permission is denied, stop and say it was denied.');
    await page.getByRole('button', { name: 'Send message', exact: true }).click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible({ timeout: 180000 });
    await expect(dialog.getByText('read_wiki_structure', { exact: true })).toBeVisible();
    const args = JSON.parse(await dialog.locator('pre').innerText());
    expect(args.repoName).toBe('tauri-apps/tauri');
    if (decision === 'cancel') {
      await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); await invoke('cancel_generation'); });
    } else {
      await dialog.getByRole('button', { name: decision === 'allow' ? 'Allow once' : 'Deny', exact: true }).click();
    }
    await expect(dialog).toHaveCount(0, { timeout: 20000 });
    await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
    await expect(page.getByRole('alert')).toHaveCount(0);
    const audits = await page.evaluate(async () => {
      const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
      const state = await invoke('bootstrap');
      const messages = await invoke('get_messages', { id: state.conversations[0].id });
      return messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    });
    const answer = await page.locator('.message-assistant .markdown').innerText();
    if (decision === 'allow') { expect(audits[0].request.decision).toBe('allowed'); expect(audits[0].result.isError).not.toBe(true); expect(JSON.stringify(audits[0].result)).toContain('Overview'); expect(answer.length).toBeGreaterThan(10); }
    if (decision === 'deny') { expect(audits[0].request.decision).toBe('denied'); expect(audits[0].result.isError).toBe(true); expect(answer.toLowerCase()).toContain('denied'); }
    if (decision === 'cancel') { expect(audits.length).toBe(0); await expect(page.getByText('Response stopped', { exact: true })).toBeVisible(); }
    report.scenarios.push({ decision, args, answer, auditCount: audits.length });
  }
  mkdirSync('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/native-agent.png' });
  writeFileSync('test-results/agent-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
