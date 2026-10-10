import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { writeFileSync } from 'node:fs';

const [serverPath, modelPath, outputPath, contextsArg = '16384,131072', profile = 'f16'] = process.argv.slice(2);
if (!serverPath || !modelPath || !outputPath) {
  console.error('usage: node scripts/runtime-context-benchmark.mjs <llama-server.exe> <model.gguf> <output.json> [contexts] [f16|saved]');
  process.exit(2);
}
const contexts = contextsArg.split(',').map(Number);
if (contexts.length < 1 || contexts.some(context => !Number.isInteger(context) || context < 128 || context > 131_072)) {
  console.error('contexts must be comma-separated integers between 128 and 131072');
  process.exit(2);
}
if (!['f16', 'saved'].includes(profile)) {
  console.error('profile must be f16 or saved');
  process.exit(2);
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function freePort() {
  const server = createServer();
  await new Promise((resolve, reject) => server.once('error', reject).listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

async function benchmark(contextLength) {
  const port = await freePort();
  const env = { ...process.env };
  delete env.LLAMA_API_KEY;
  const args = ['--model', modelPath, '--host', '127.0.0.1', '--port', String(port), '--no-webui',
    '--ctx-size', String(contextLength), '--fit', 'on', '--threads', '6', '--parallel', '1',
    '--batch-size', profile === 'saved' ? '512' : '2048', '--ubatch-size', profile === 'saved' ? '128' : '512',
    '--cache-type-k', profile === 'saved' ? 'q8_0' : 'f16', '--cache-type-v', profile === 'saved' ? 'q8_0' : 'f16',
    '--flash-attn', profile === 'saved' ? 'on' : 'off'];
  const child = spawn(serverPath, args, { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let log = '';
  let spawnError = null;
  child.on('error', error => { spawnError = error; });
  for (const stream of [child.stdout, child.stderr]) stream.on('data', chunk => { log = (log + chunk.toString()).slice(-250_000); });
  const started = Date.now();
  try {
    let ready = false;
    while (Date.now() - started < 180_000 && child.exitCode === null && !spawnError) {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2_000) });
        if (response.ok) { ready = true; break; }
      } catch { /* loading */ }
      await sleep(1_000);
    }
    if (!ready) throw new Error(`llama-server did not become ready (exit=${child.exitCode}, spawn=${spawnError})`);
    const startupMs = Date.now() - started;
    const prompt = 'State one practical reason to verify numerical calculations with an independent formula. '.repeat(35);
    const response = await fetch(`http://127.0.0.1:${port}/completion`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ prompt, n_predict: 128, temperature: 0, cache_prompt: false }),
      signal: AbortSignal.timeout(120_000),
    });
    if (!response.ok) throw new Error(`completion HTTP ${response.status}: ${(await response.text()).slice(0, 500)}`);
    const result = await response.json();
    const offload = [...log.matchAll(/offloaded\s+(\d+)\s*\/\s*(\d+)\s+layers?/gi)].at(-1);
    return {
      contextLength, startupMs,
      offloadedLayers: offload ? Number(offload[1]) : null,
      totalLayers: offload ? Number(offload[2]) : null,
      timings: result.timings ?? null,
      tokensPredicted: result.tokens_predicted ?? null,
      error: null,
    };
  } catch (error) {
    return { contextLength, error: String(error), logTail: log.slice(-4_000) };
  } finally {
    child.kill();
    if (child.exitCode === null) await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(10_000)]);
  }
}

const results = [];
for (const contextLength of contexts) {
  console.log(`benchmark context=${contextLength}`);
  const result = await benchmark(contextLength);
  results.push(result);
  console.log(JSON.stringify(result));
}
writeFileSync(outputPath, JSON.stringify({ serverPath, modelPath, profile, recordedAt: new Date().toISOString(), results }, null, 2));
if (results.some(result => result.error)) process.exitCode = 1;
