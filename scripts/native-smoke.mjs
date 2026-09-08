import { chromium, expect } from '@playwright/test';
import { resolve } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';

// Launch the debug Tauri binary with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223 first.
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(page => page.url().includes('1420'));
if (!page) throw new Error('LocalLM native webview is not running.');
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await expect(page.getByText('Browser preview', { exact: false })).toHaveCount(0);
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('textbox', { name: 'llama-server executable' }).fill(resolve('.local/runtime/llama-server.exe'));
  await page.getByRole('textbox', { name: 'GGUF model file' }).fill(resolve('.local/models/MiniCPM5-2B.Q6_K.gguf'));
  await page.getByRole('button', { name: 'Save settings' }).click();
  await expect(page.getByText('Settings saved.', { exact: true })).toBeVisible();
  await page.getByRole('tab', { name: 'Generation', exact: true }).click();
  await page.getByRole('spinbutton', { name: 'Maximum response tokens' }).fill('512');
  await page.getByRole('button', { name: 'Save settings' }).click();
  const started = Date.now();
  await page.getByRole('button', { name: 'Load model', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  const loadMs = Date.now() - started;
  await page.getByRole('button', { name: 'New conversation', exact: false }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('What is 17 + 25? Answer briefly.');
  const generationStarted = Date.now();
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop response' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  const response = await page.locator('.message-assistant .markdown').innerText();
  if (!response.includes('42')) throw new Error(`Unexpected arithmetic response: ${response}`);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.getByRole('button', { name: 'Rename conversation', exact: true }).click();
  await page.getByRole('textbox', { name: 'Conversation title' }).fill('GPU smoke test');
  await page.getByRole('button', { name: 'Save title' }).click();
  await expect(page.locator('.workspace-title')).toHaveText('GPU smoke test');
  await page.reload();
  await page.getByRole('button', { name: 'GPU smoke test', exact: true }).click();
  await expect(page.locator('.message-assistant .markdown')).toContainText('42');
  mkdirSync('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/native-chat.png' });
  if (errors.length) throw new Error(`Webview errors: ${errors.join('\n')}`);
  const report = { testedAt: new Date().toISOString(), loadMs, generationMs: Date.now() - generationStarted, response, errors, persistence: 'verified after webview reload' };
  writeFileSync('test-results/native-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  await browser.close();
}
