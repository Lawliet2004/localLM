// Real Windows/CUDA acceptance; requires the verified Bonsai model and Prism runtime.
// node scripts/bonsai-runtime-smoke.mjs [runtime.exe] [model.gguf] [context]
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { resolve, join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

const runtime = resolve(process.argv[2] ?? '.local/runtime-prism-b9601-68faa14/llama-server.exe');
const model = resolve(process.argv[3] ?? join(process.env.APPDATA, 'app.locallm.desktop/models/Ternary-Bonsai-8B-Q2_0.gguf'));
const context = Number(process.argv[4] ?? 4096);
assert.ok(Number.isInteger(context) && context >= 128 && context <= 65536, 'Invalid Bonsai context');
const listener = createServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
const endpoint = `http://127.0.0.1:${port}`;
const key = randomUUID();
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('LLAMA_')));
env.LLAMA_API_KEY = key;
const help = execFileSync(runtime, ['--help'], { env, windowsHide: true, timeout: 10_000, encoding: 'utf8' });
const child = spawn(runtime, [
  '--model', model, '--ctx-size', String(context), '--n-gpu-layers', '-1',
  '--threads', '6', '--batch-size', '512', '--ubatch-size', '128',
  '--flash-attn', 'on', '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0',
  '--host', '127.0.0.1', '--port', String(port), '--parallel', '1',
  '--jinja', '--no-webui', ...(help.includes('--no-agent') ? ['--no-agent'] : []), '--fit', 'off', '--log-verbosity', '4',
], { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let log = '';
let spawnError;
child.on('error', error => { spawnError = error; });
for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { if (log.length < 2_000_000) log += chunk; });
const headers = { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' };
const started = Date.now();
try {
  let ready = false;
  while (Date.now() - started < 120_000) {
    if (spawnError) throw spawnError;
    assert.equal(child.exitCode, null, `Runtime exited: ${log.slice(-6000)}`);
    try {
      const health = await fetch(`${endpoint}/health`, { headers, signal: AbortSignal.timeout(2000) });
      if (health.ok) { ready = true; break; }
    } catch { /* Process may still be loading. */ }
    await delay(500);
  }
  assert.ok(ready, `Startup timed out: ${log.slice(-6000)}`);
  const offload = log.match(/offloaded (\d+)\/(\d+) layers to GPU/);
  assert.ok(offload && Number(offload[1]) > 0, 'No measured GPU layer offload');
  const response = await fetch(`${endpoint}/v1/chat/completions`, {
    method: 'POST', headers, signal: AbortSignal.timeout(120_000),
    body: JSON.stringify({ messages: [{ role: 'user', content: 'What is 17 + 25? Answer with just the number. /no_think' }], max_tokens: 128, temperature: 0, chat_template_kwargs: { enable_thinking: false } }),
  });
  const result = await response.json();
  assert.ok(response.ok, JSON.stringify(result));
  const answer = result.choices?.[0]?.message?.content;
  assert.match(answer ?? '', /\b42\b/, JSON.stringify(result));
  console.log(JSON.stringify({ runtime, model, context, offloadedLayers: `${offload[1]}/${offload[2]}`, answer, elapsedMs: Date.now() - started }));
} finally {
  child.kill();
  mkdirSync('.local', { recursive: true });
  writeFileSync('.local/bonsai-runtime-smoke.log', log);
}
