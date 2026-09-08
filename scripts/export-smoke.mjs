import { chromium, expect } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(page => page.url().includes('1420')); return Boolean(page); }).toBe(true);
async function invoke(command, args = {}) {
  return page.evaluate(async ({ command, args }) => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command, args); }, { command, args });
}
try {
  const state = await invoke('bootstrap');
  let selected;
  let messages;
  for (const conversation of state.conversations) {
    const rows = await invoke('get_messages', { id: conversation.id });
    if (rows.some(row => row.role === 'tool')) { selected = conversation; messages = rows; break; }
  }
  if (!selected) throw new Error('Run agent-smoke.mjs first to create a conversation with a real tool audit.');
  const directory = resolve('.local/export-smoke'); mkdirSync(directory, { recursive: true });
  const jsonPath = resolve(directory, 'conversation.json');
  const markdownPath = resolve(directory, 'conversation.md');
  await invoke('export_conversation', { id: selected.id, path: jsonPath });
  await invoke('export_conversation', { id: selected.id, path: markdownPath });
  const exported = JSON.parse(readFileSync(jsonPath, 'utf8'));
  expect(exported.schemaVersion).toBe(1);
  expect(exported.messages).toEqual(messages);
  expect(exported.conversation).toEqual(selected);
  expect(exported.toolSelection).toEqual(await invoke('get_conversation_tools', { id: selected.id }));
  const markdown = readFileSync(markdownPath, 'utf8');
  for (const message of messages) { expect(markdown).toContain(message.content); if (message.reasoning) expect(markdown).toContain(message.reasoning); }
  writeFileSync(jsonPath, 'existing export');
  await invoke('export_conversation', { id: selected.id, path: jsonPath });
  expect(JSON.parse(readFileSync(jsonPath, 'utf8')).messages).toEqual(messages);
  const report = { testedAt: new Date().toISOString(), messageCount: messages.length, toolAuditCount: messages.filter(message => message.role === 'tool').length, jsonMatchesDatabase: true, markdownIncludesAllContent: true, replacement: true };
  writeFileSync('test-results/export-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
