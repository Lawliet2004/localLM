import { chromium, expect } from '@playwright/test';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
try {
  const page = browser.contexts()[0].pages().find(page => page.url().includes('1420'));
  const result = await page.evaluate(async () => {
    const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
    const connectors = await invoke('list_connectors');
    const alreadyConnected = connectors.some(item => item.id === 'deepwiki' && item.connected);
    if (!alreadyConnected) await invoke('connect_connector', { id: 'deepwiki' });
    const chat = await invoke('create_conversation');
    try {
      let error = '';
      try { await invoke('send_message', { conversationId: chat.id, content: 'This must not be persisted', connectorIds: [], connectorTools: [{ connectorId: 'deepwiki', toolName: 'missing_tool' }], channel: new Channel() }); }
      catch (e) { error = String(e); }
      return { error, messages: await invoke('get_messages', { id: chat.id }) };
    } finally {
      await invoke('delete_conversation', { id: chat.id });
      if (!alreadyConnected) await invoke('disconnect_connector', { id: 'deepwiki', forget: false });
    }
  });
  expect(result.error).toContain('no longer available');
  expect(result.messages).toHaveLength(0);
  console.log(JSON.stringify(result));
} finally { await browser.close(); }
