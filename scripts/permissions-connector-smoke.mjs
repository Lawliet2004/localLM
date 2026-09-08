import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(page => page.url().includes('1420'));
try {
  const report = await page.evaluate(async () => {
    const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
    const originalPreferences = (await invoke('bootstrap')).preferences;
    const connected = (await invoke('list_connectors')).some(item => item.id === 'deepwiki' && item.connected);
    const results = [];
    try {
      await invoke('save_preferences', { preferences: { ...originalPreferences, temperature: 0, maxTokens: 2048 } });
      if (!connected) await invoke('connect_connector', { id: 'deepwiki' });
      for (const mode of ['autoApprove', 'fullAccess']) {
        const chat = await invoke('create_conversation');
        try {
          const tools = [{ connectorId: 'deepwiki', toolName: 'read_wiki_structure' }];
          await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: mode } });
          let prompts = 0;
          const decisions = [];
          const channel = new Channel();
          channel.onmessage = event => { if (event.approval) { prompts++; decisions.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: false })); } };
          await invoke('send_message', { conversationId: chat.id, content: 'Use read_wiki_structure once for tauri-apps/tauri. If denied, stop. Otherwise name one section from the result.', connectorIds: [], connectorTools: tools, channel });
          await Promise.all(decisions);
          const audits = (await invoke('get_messages', { id: chat.id })).filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
          results.push({ mode, prompts, audits });
        } finally { await invoke('delete_conversation', { id: chat.id }); }
      }
      return results;
    } finally {
      await invoke('save_preferences', { preferences: originalPreferences });
      if (!connected) await invoke('disconnect_connector', { id: 'deepwiki', forget: false });
    }
  });
  expect(report[0].prompts).toBe(1);
  expect(report[0].audits[0].request.decision).toBe('denied');
  expect(report[1].prompts).toBe(0);
  expect(report[1].audits).toHaveLength(1);
  expect(report[1].audits[0].request.authorization).toBe('full access selected by user');
  expect(report[1].audits[0].result.isError).not.toBe(true);
  expect(JSON.stringify(report[1].audits[0].result)).toContain('Overview');
  const summary = report.map(item => ({ mode: item.mode, prompts: item.prompts, decision: item.audits[0].request.decision }));
  writeFileSync('test-results/permissions-connector-smoke.json', JSON.stringify(summary, null, 2));
  console.log(JSON.stringify(summary));
} finally { await browser.close(); }
