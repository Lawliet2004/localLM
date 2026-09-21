// Real Windows/CUDA acceptance for Ternary Bonsai 2 PTQ1_0 + Prism b10709.
// node scripts/bonsai2-runtime-smoke.mjs [runtime.exe] [model.gguf] [context]
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { resolve, join } from 'node:path';
import { mkdirSync, writeFileSync } from 'node:fs';
import { setTimeout as delay } from 'node:timers/promises';

const runtime = resolve(process.argv[2] ?? '.local/runtime-prism-b10709-9a9394a/llama-server.exe');
const defaultModel = join(
  process.env.APPDATA,
  'app.locallm.desktop/models/1c4401849640b8a18a6c2be9369953de0d52594d75d38d0bb573d3867a904114/Ternary-Bonsai-2-27B-PTQ1_0.gguf',
);
const model = resolve(process.argv[3] ?? defaultModel);
const context = Number(process.argv[4] ?? 4096);
assert.ok(Number.isInteger(context) && context >= 128 && context <= 262144, 'Invalid Bonsai 2 context');
const listener = createServer();
await new Promise(resolveListener => listener.listen(0, '127.0.0.1', resolveListener));
const port = listener.address().port;
await new Promise(resolveClose => listener.close(resolveClose));
const endpoint = `http://127.0.0.1:${port}`;
const key = randomUUID();
const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('LLAMA_')));
env.LLAMA_API_KEY = key;
const help = execFileSync(runtime, ['--help'], { env, windowsHide: true, timeout: 10_000, encoding: 'utf8' });
const args = [
  '--model', model, '--ctx-size', String(context),
  '--threads', '6', '--batch-size', '256', '--ubatch-size', '64',
  '--flash-attn', 'on', '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0',
  '--host', '127.0.0.1', '--port', String(port), '--parallel', '1',
  '--jinja', '--no-webui', '--log-verbosity', '4',
];
if (help.includes('--no-agent')) args.push('--no-agent');
if (help.includes('--no-mmproj')) args.push('--no-mmproj');
if (help.includes('--fit')) args.push('--fit', 'on');
else args.push('--n-gpu-layers', '99');
const child = spawn(runtime, args, { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
let log = '';
let spawnError;
child.on('error', error => { spawnError = error; });
for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { if (log.length < 2_000_000) log += chunk; });
const headers = { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' };
const started = Date.now();
try {
  let ready = false;
  while (Date.now() - started < 600_000) {
    if (spawnError) throw spawnError;
    assert.equal(child.exitCode, null, `Runtime exited: ${log.slice(-8000)}`);
    try {
      const health = await fetch(`${endpoint}/health`, { headers, signal: AbortSignal.timeout(2000) });
      if (health.ok) { ready = true; break; }
    } catch { /* Process may still be loading. */ }
    await delay(500);
  }
  assert.ok(ready, `Startup timed out: ${log.slice(-8000)}`);
  assert.doesNotMatch(log, /invalid ggml type/, log.slice(-8000));
  const offload = log.match(/offloaded (\d+)\/(\d+) layers to GPU/);
  const response = await fetch(`${endpoint}/v1/chat/completions`, {
    method: 'POST', headers, signal: AbortSignal.timeout(180_000),
    body: JSON.stringify({
      messages: [{ role: 'user', content: 'What is 17 + 25? Answer with just the number. /no_think' }],
      max_tokens: 64, temperature: 0,
      chat_template_kwargs: { enable_thinking: false },
    }),
  });
  const result = await response.json();
  assert.ok(response.ok, JSON.stringify(result));
  const answer = result.choices?.[0]?.message?.content;
  assert.match(answer ?? '', /\b42\b/, JSON.stringify(result));
  console.log(JSON.stringify({
    runtime, model, context,
    offloadedLayers: offload ? `${offload[1]}/${offload[2]}` : null,
    answer, elapsedMs: Date.now() - started,
  }));
} finally {
  child.kill();
  mkdirSync('.local', { recursive: true });
  writeFileSync('.local/bonsai2-runtime-smoke.log', log);
}
