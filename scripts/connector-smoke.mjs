import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

// Run against a debug Tauri process with WebView2 CDP enabled on port 9223.
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(value => value.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
try {
  await page.getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search connectors' }).fill('deepwiki');
  const item = page.locator('.catalog-list .catalog-item');
  await expect(item).toHaveCount(1);
  if (await item.getAttribute('open') === null) await item.locator('summary').first().click();
  await item.getByRole('button', { name: 'Connect', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Disconnect', exact: true })).toBeVisible({ timeout: 70000 });
  const toolNames = await item.locator('.connector-tools summary').allTextContents();
  expect(toolNames).toContain('ask_question');
  expect(toolNames).toContain('read_wiki_structure');
  await expect(page.getByRole('alert')).toHaveCount(0);
  mkdirSync('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/native-connectors.png' });
  await item.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
  const report = { testedAt: new Date().toISOString(), service: 'https://mcp.deepwiki.com/mcp', toolNames, disconnect: 'verified' };
  writeFileSync('test-results/connector-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
