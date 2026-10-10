import { chromium } from '@playwright/test';
const b = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = b.contexts()[0].pages().find(p => p.url().includes('1420'));
const r = await Promise.race([
  page.evaluate(async () => {
    const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
    const s = await invoke('bootstrap');
    const e = window.__eval ? { done: window.__eval.done, events: window.__eval.events.length } : null;
    const convo = s.conversations[0];
    let run = null, nmsgs = 0, toolNames = [];
    if (convo) {
      run = await invoke('get_conversation_run', { conversationId: convo.id });
      const msgs = await invoke('get_messages', { id: convo.id });
      nmsgs = msgs.length;
      toolNames = msgs.filter(m => m.role === 'tool').map(m => { try { return JSON.parse(m.content).request?.name } catch { return '?' } });
    }
    return { eval: e, convoTitle: convo?.title?.slice(0, 60), run: run && { status: run.status, error: run.error, updatedAt: run.updatedAt }, nmsgs, toolNames, runtime: s.runtime?.phase, model: s.runtime?.modelPath?.split(/[\\/]/).pop() };
  }),
  new Promise(r => setTimeout(() => r({ timeout: 'evaluate hung' }), 20000))
]);
console.log(JSON.stringify(r, null, 1));
await b.close();
