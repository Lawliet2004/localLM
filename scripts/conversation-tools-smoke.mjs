import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(page => page.url().includes('1420'));
page.setDefaultTimeout(15000);
let chats = [];
const suffix = Date.now();
async function invoke(command, args = {}) {
  return page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
}
async function openTools() {
  const picker = page.locator('.tool-picker');
  if (await picker.getAttribute('open') === null) await picker.locator('summary').first().click();
  const group = picker.locator('.connector-tool-group').filter({ hasText: 'deepwiki' });
  if (await group.getAttribute('open') === null) await group.locator('summary').click();
  return group;
}
try {
  await invoke('connect_connector', { id: 'deepwiki' });
  for (const name of ['A', 'B']) {
    const chat = await invoke('create_conversation'); chats.push(chat);
    await invoke('rename_conversation', { id: chat.id, title: `Tool settings ${name} ${suffix}` });
  }
  await page.reload();
  await page.getByRole('button', { name: `Tool settings A ${suffix}`, exact: true }).click();
  let group = await openTools();
  await group.getByRole('checkbox', { name: 'read_wiki_structure', exact: true }).click();
  await expect.poll(() => invoke('get_conversation_tools', { id: chats[0].id })).toEqual({ accessMode: 'ask', sources: [], tools: [{ connectorId: 'deepwiki', toolName: 'read_wiki_structure' }] });
  await page.getByRole('button', { name: `Tool settings B ${suffix}`, exact: true }).click();
  await expect(group.getByRole('checkbox', { name: 'read_wiki_structure', exact: true })).not.toBeChecked();
  await group.getByRole('checkbox', { name: 'ask_question', exact: true }).click();
  await expect.poll(() => invoke('get_conversation_tools', { id: chats[1].id })).toEqual({ accessMode: 'ask', sources: [], tools: [{ connectorId: 'deepwiki', toolName: 'ask_question' }] });
  await page.reload();
  await page.getByRole('button', { name: `Tool settings A ${suffix}`, exact: true }).click();
  group = await openTools();
  await expect(group.getByRole('checkbox', { name: 'read_wiki_structure', exact: true })).toBeChecked();
  await expect(group.getByRole('checkbox', { name: 'ask_question', exact: true })).not.toBeChecked();
  await page.getByRole('button', { name: 'New conversation', exact: false }).click();
  await expect(page.locator('.tool-picker > summary')).toHaveText('Tools · Off');
  await expect(group.getByRole('checkbox', { name: 'read_wiki_structure', exact: true })).not.toBeChecked();
  const report = { testedAt: new Date().toISOString(), isolation: true, reload: true, newChatToolsOff: true };
  writeFileSync('test-results/conversation-tools-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  for (const chat of chats) await invoke('delete_conversation', { id: chat.id });
  await page.reload();
  await browser.close();
}
