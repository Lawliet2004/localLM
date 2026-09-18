// Real checkpoint acceptance: node scripts/zaya-runtime-smoke.mjs [llama-server.exe]
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { resolve, join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

const runtime = resolve(process.argv[2] ?? '.local/llama.cpp-3750f9ce7ac20f7a905b43d9f20ad1050884f6c7/build/bin/Release/llama-server.exe');
const model = join(process.env.APPDATA, 'app.locallm.desktop/models/ZAYA1-8B-Q4_K_M.gguf');
const listener = createServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
const endpoint = `http://127.0.0.1:${port}`;
const key = randomUUID();
const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('LLAMA_')));
env.LLAMA_API_KEY = key;
const child = spawn(runtime, [
  '--model', model, '--ctx-size', '8192', '--n-gpu-layers', '0',
  '--threads', '6', '--batch-size', '128', '--ubatch-size', '32',
  '--flash-attn', 'off', '--cache-type-k', 'f16', '--cache-type-v', 'f16',
  '--host', '127.0.0.1', '--port', String(port), '--parallel', '1',
  '--jinja', '--no-webui', '--fit', 'off', '--log-verbosity', '4',
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
      ready = (await fetch(`${endpoint}/v1/models`, { headers, signal: AbortSignal.timeout(2000) })).ok;
      if (ready) break;
    } catch { /* Runtime is still starting. */ }
    await delay(500);
  }
  assert.ok(ready, `Startup timed out: ${log.slice(-6000)}`);
  const payload = { messages: [{ role: 'user', content: 'What is 17 + 25? Answer briefly.' }], max_tokens: 8192, reasoning_budget_tokens: 2048, temperature: 0 };
  const tokens = await fetch(`${endpoint}/v1/chat/completions/input_tokens`, {
    method: 'POST', headers, body: JSON.stringify(payload), signal: AbortSignal.timeout(10_000),
  });
  assert.ok(tokens.ok, `App context preflight failed: ${await tokens.text()}`);
  const response = await fetch(`${endpoint}/v1/chat/completions`, {
    method: 'POST', headers, body: JSON.stringify(payload), signal: AbortSignal.timeout(180_000),
  });
  const result = await response.json();
  assert.ok(response.ok, JSON.stringify(result));
  const answer = result.choices?.[0]?.message?.content;
  assert.match(answer ?? '', /\b42\b/, JSON.stringify(result));
  assert.equal(result.choices[0].finish_reason, 'stop', 'Model exhausted its response budget');
  const evidence = { runtime, model, context: 8192, backend: 'CPU', answer, elapsedMs: Date.now() - started, usage: result.usage };
  mkdirSync('test-results', { recursive: true });
  writeFileSync('test-results/zaya-runtime-smoke.json', JSON.stringify(evidence, null, 2));
  console.log(JSON.stringify(evidence));
} finally {
  child.kill();
  mkdirSync('.local', { recursive: true });
  writeFileSync('.local/zaya-runtime-smoke.log', log);
}
