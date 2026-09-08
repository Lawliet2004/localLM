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
  const title = `Retry acceptance ${randomUUID().slice(0, 8)}`;
  await invoke('rename_conversation', { id: chat.id, title });
  await page.reload();
  await page.getByRole('button', { name: title, exact: true }).click();
  const prompt = 'What is 17 + 25? Answer briefly.';
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill(prompt);
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByLabel('Last request context')).toBeVisible();
  await page.getByRole('button', { name: 'Stop response' }).click();
  await expect(page.getByRole('button', { name: 'Retry last prompt' })).toBeVisible();
  const first = await invoke('get_messages', { id: chat.id });
  expect(first.at(-1).status).toBe('interrupted');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this unsent draft');
  await page.getByRole('button', { name: 'Retry last prompt' }).click();
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  const final = await invoke('get_messages', { id: chat.id });
  expect(final.slice(0, first.length)).toEqual(first);
  expect(final.filter(message => message.role === 'user').map(message => message.content)).toEqual([prompt, prompt]);
  expect(final.at(-1).status).toBe('complete');
  expect(final.at(-1).content).toContain('42');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this unsent draft');
  writeFileSync('test-results/retry-smoke.json', JSON.stringify({ testedAt: new Date().toISOString(), originalRetained: true, newAttemptCompleted: true, draftPreserved: true, messages: final.length }, null, 2));
} finally {
  await invoke('cancel_generation');
  await expect.poll(async () => { try { await invoke('save_preferences', { preferences: original.preferences }); return true; } catch { return false; } }, { timeout: 30000 }).toBe(true);
  if (chat) { await invoke('delete_conversation', { id: chat.id }); await page.evaluate(id => localStorage.removeItem(`locallm-draft:${id}`), chat.id); }
  if (original.runtime.phase !== 'ready') await invoke('unload_model');
  await page.reload(); await browser.close();
}
