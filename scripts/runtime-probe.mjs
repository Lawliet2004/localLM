import { spawn } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { createServer } from 'node:net';
import { createWriteStream, mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

mkdirSync('.local', { recursive: true });
mkdirSync('test-results', { recursive: true });
const listener = createServer();
await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise(resolve => listener.close(resolve));
const key = randomUUID();
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('LLAMA_')));
const child = spawn(resolve('.local/runtime/llama-server.exe'), [
  '--model', resolve('.local/models/MiniCPM5-2B.Q6_K.gguf'), '--host', '127.0.0.1', '--port', String(port),
  '--ctx-size', '8192', '--n-gpu-layers', '-1', '--threads', '6', '--batch-size', '512', '--ubatch-size', '128',
  '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0', '--flash-attn', 'on', '--parallel', '1', '--fit', 'off',
  '--jinja', '--no-webui', '--no-agent', '--log-verbosity', '3',
], { windowsHide: true, env: { ...env, LLAMA_API_KEY: key }, stdio: ['ignore', 'pipe', 'pipe'] });
const log = createWriteStream('.local/probe.log');
child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false });
let launchError;
child.on('error', error => { launchError = error; });
const endpoint = `http://127.0.0.1:${port}`;
async function completion(body) {
  const response = await fetch(`${endpoint}/v1/chat/completions`, {
    method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${key}` },
    body: JSON.stringify({ temperature: 0, max_tokens: 768, ...body }), signal: AbortSignal.timeout(120000),
  });
  if (!response.ok) throw new Error(`Inference ${response.status}: ${await response.text()}`);
  return response.json();
}
try {
  let ready = false;
  for (let i = 0; i < 120; i++) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`Runtime exited: ${child.exitCode}`);
    try { ready = (await fetch(`${endpoint}/health`, { signal: AbortSignal.timeout(1500) })).ok; } catch {}
    if (ready) break;
    await delay(500);
  }
  if (!ready) throw new Error('Runtime did not become ready.');
  const tools = [{ type: 'function', function: { name: 'get_weather', description: 'Get the current temperature for a city.', parameters: { type: 'object', properties: { city: { type: 'string' } }, required: ['city'], additionalProperties: false } } }];
  const messages = [{ role: 'user', content: 'Use get_weather to find the temperature in Paris. Do not guess; call the tool first.' }];
  const first = await completion({ messages, tools });
  const assistant = first.choices[0].message;
  const calls = assistant.tool_calls;
  if (!Array.isArray(calls) || calls.length !== 1 || calls[0].function.name !== 'get_weather') {
    writeFileSync('test-results/tool-probe-failure.json', JSON.stringify(first, null, 2));
    throw new Error('Runtime did not return one structured get_weather call. Inspect test-results/tool-probe-failure.json.');
  }
  const args = JSON.parse(calls[0].function.arguments);
  if (args.city.toLowerCase() !== 'paris') throw new Error(`Incorrect tool argument: ${JSON.stringify(args)}`);
  const second = await completion({ messages: [...messages, assistant, { role: 'tool', tool_call_id: calls[0].id, content: JSON.stringify({ city: 'Paris', temperature_c: 19, source: 'synthetic integration fixture' }) }], tools });
  if (!second.choices[0].message.content?.includes('19')) throw new Error('Tool result was not incorporated in the final response.');
  const report = { testedAt: new Date().toISOString(), fixture: 'Synthetic weather; no external service contacted', first, second };
  writeFileSync('test-results/tool-probe.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ tool: calls[0].function, final: second.choices[0].message.content, timings: second.timings }));
} finally {
  child.kill();
  await new Promise(resolve => child.exitCode !== null ? resolve() : child.once('exit', resolve));
  log.end();
}
