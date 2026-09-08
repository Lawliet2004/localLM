import { chromium, expect } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
const invoke = (command, args = {}) => page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
const original = await invoke('bootstrap');
let chat;
try {
  await invoke('save_preferences', { preferences: { ...original.preferences, maxTokens: 512, temperature: 0 } });
  if (original.runtime.phase !== 'ready') await invoke('load_model');
  chat = await invoke('create_conversation');
  const title = `Context indicator ${randomUUID().slice(0, 8)}`;
  await invoke('rename_conversation', { id: chat.id, title });
  await page.reload();
  await page.getByRole('button', { name: title, exact: true }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('What is 17 + 25? Answer briefly.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const indicator = page.getByLabel('Last request context');
  await expect(indicator).toBeVisible({ timeout: 30000 });
  const text = await indicator.innerText();
  expect(text).toContain('512 response reserve');
  expect(text).toContain('Draft changes are not included');
  expect(Number(text.match(/Last request: ([\d,]+)/)[1].replaceAll(',', ''))).toBeGreaterThan(0);
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  await expect(page.locator('.message-assistant .markdown')).toContainText('42');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Unsent draft');
  expect(await indicator.innerText()).toBe(text);
  await page.getByRole('button', { name: 'New conversation', exact: false }).click();
  await expect(indicator).toHaveCount(0);
  writeFileSync('test-results/context-indicator-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), text, answer: '42', draftDoesNotChangeCount: true, isolatedFromNewConversation: true }, null, 2));
} finally {
  await invoke('cancel_generation');
  await expect.poll(async () => { try { await invoke('save_preferences', { preferences: original.preferences }); return true; } catch { return false; } }, { timeout: 30000 }).toBe(true);
  if (chat) { await invoke('delete_conversation', { id: chat.id }); await page.evaluate(id => localStorage.removeItem(`locallm-draft:${id}`), chat.id); }
  if (original.runtime.phase !== 'ready') await invoke('unload_model');
  await page.reload();
  await browser.close();
}
