// Configure and verify ZAYA in the native development app; retain it on success.
// Requires the runtime smoke to pass and the app's local CDP port to be enabled.
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const evidence = JSON.parse(readFileSync('test-results/zaya-runtime-smoke.json', 'utf8'));
// Attach to the page directly: this WebView's shared-worker targets omit the
// browserContextId required by Playwright's browser-wide attachment.
const targets = await (await fetch('http://127.0.0.1:9223/json/list')).json();
const target = targets.find(target => target.type === 'page' && target.url.includes(':1420'));
assert.ok(target, 'Native development app is not open');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
let sequence = 0;
const pending = new Map();
socket.onmessage = event => {
  const message = JSON.parse(event.data);
  const entry = pending.get(message.id);
  if (!entry) return;
  pending.delete(message.id);
  clearTimeout(entry.timer);
  message.error ? entry.reject(new Error(JSON.stringify(message.error))) : entry.resolve(message.result);
};
function cdp(method, params = {}) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 240_000);
    pending.set(id, { resolve, reject, timer });
    socket.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(fn, arg) {
  const result = await cdp('Runtime.evaluate', { expression: `(${fn})(${JSON.stringify(arg) ?? 'undefined'})`, awaitPromise: true, returnByValue: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
  return result.result.value;
}
const invoke = (command, args = {}) => evaluate(async ({ command, args }) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command, args);
}, { command, args });
const original = await invoke('bootstrap');
mkdirSync('.local', { recursive: true });
const backupPath = `.local/zaya-settings-before-${Date.now()}.json`;
writeFileSync(backupPath, JSON.stringify({ preferences: original.preferences, config: original.config }, null, 2));
let id;
let passed = false;
try {
  const config = { ...original.config, contextLength: 8192, gpuLayers: 0, cpuThreads: 6,
    batchSize: 128, microBatchSize: 32, flashAttention: false,
    cacheTypeK: 'f16', cacheTypeV: 'f16', offloadKvCache: false, mmap: true };
  const preferences = { ...original.preferences, maxTokens: Math.max(2048, original.preferences.maxTokens), runtimePath: evidence.runtime,
    modelPath: join(process.env.APPDATA, 'app.locallm.desktop/models/ZAYA1-8B-Q4_K_M.gguf') };
  await invoke('save_runtime_config', { config });
  await invoke('save_preferences', { preferences });
  assert.equal((await invoke('load_model')).phase, 'ready');
  id = (await invoke('create_conversation')).id;
  await invoke('rename_conversation', { id, title: 'ZAYA1 setup check' });
  await invoke('save_conversation_model', { id, selection: { providerId: null, modelId: '' } });
  await invoke('set_preset', { conversationId: id, preset: 'chat' });
  await evaluate(async id => {
    const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
    await invoke('send_message', { conversationId: id, content: 'What is 17 + 25? Answer briefly.',
      connectorIds: [], connectorTools: [], channel: new Channel() });
  }, id);
  const messages = await invoke('get_messages', { id });
  const answer = messages.findLast(message => message.role === 'assistant');
  assert.equal(answer.status, 'complete');
  assert.match(answer.content, /\b42\b/);
  await invoke('rename_conversation', { id, title: 'ZAYA1 setup check' });
  await cdp('Page.reload');
  let visible = false;
  for (let attempt = 0; attempt < 40; attempt++) {
    await delay(250);
    try {
      visible = await evaluate(() => {
        document.querySelector('button[aria-label="Expand sidebar"]')?.click();
        const button = [...document.querySelectorAll('button')].find(button => button.textContent.trim() === 'ZAYA1 setup check');
        if (document.querySelector('.workspace-title')?.textContent !== 'ZAYA1 setup check') button?.click();
        return /\b42\b/.test(document.querySelector('.message-assistant .markdown')?.textContent ?? '');
      });
      if (visible) break;
    } catch { /* Page may still be reloading. */ }
  }
  assert.ok(visible, 'Persisted answer was not visible after reload');
  const screenshot = await cdp('Page.captureScreenshot', { format: 'png' });
  writeFileSync('.local/zaya-app-verification.png', Buffer.from(screenshot.data, 'base64'));
  const report = { runtime: evidence.runtime, model: preferences.modelPath, config, answer: answer.content, backupPath, conversationId: id };
  writeFileSync('test-results/zaya-app-smoke.json', JSON.stringify(report, null, 2));
  passed = true;
  console.log(JSON.stringify(report));
} finally {
  try {
    if (!passed) {
      await invoke('unload_model');
      await invoke('save_runtime_config', { config: original.config });
      await invoke('save_preferences', { preferences: original.preferences });
      if (id) await invoke('delete_conversation', { id });
      if (original.runtime.phase === 'ready') await invoke('load_model');
    }
  } finally {
    socket.close();
  }
}
