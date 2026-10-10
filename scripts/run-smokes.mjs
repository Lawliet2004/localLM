#!/usr/bin/env node
// Sequential runner for the *-smoke.mjs scripts listed in smokes.manifest.json.
//
//   node scripts/run-smokes.mjs [--only <name[,name...]>] [--json] [--list]
//
// Prerequisites are probed once up front; a smoke whose needs are unmet is
// reported SKIP and never fails the suite. Exit code is non-zero only on FAIL.
// Needs: debug-app (WebView2 CDP on :9223/:9224), model (GGUF or llama-server
// under .local/ or %APPDATA%\app.locallm.desktop, or LOCAL_LLM_BASE_URL),
// searxng (http://127.0.0.1:8080), docker, node (always present).

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(readFileSync(join(root, 'scripts/smokes.manifest.json'), 'utf8'));

const argv = process.argv.slice(2);
const json = argv.includes('--json');
const list = argv.includes('--list');
const onlyIdx = argv.indexOf('--only');
const only = onlyIdx >= 0 ? (argv[onlyIdx + 1] || '').split(',').filter(Boolean) : null;
const timeoutMs = Number(process.env.SMOKE_TIMEOUT_MS || 10 * 60 * 1000);

async function httpUp(url) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(2500) });
    await res.arrayBuffer().catch(() => {});
    return true;
  } catch {
    return false;
  }
}

function dirHas(dir, re) {
  try { return readdirSync(dir).some(f => re.test(f)); } catch { return false; }
}

async function detectPrereqs() {
  const appData = process.env.APPDATA ? join(process.env.APPDATA, 'app.locallm.desktop') : null;
  const cdpUrls = [process.env.LOCALLM_CDP_URL, 'http://127.0.0.1:9223', 'http://127.0.0.1:9224'].filter(Boolean);
  const checks = await Promise.all(cdpUrls.map(u => httpUp(`${u}/json/version`)));
  const debugApp = checks.some(Boolean);

  const ggufDirs = [join(root, '.local/models'), appData && join(appData, 'models')].filter(Boolean);
  const runtimeDirs = [join(root, '.local/runtime'), appData && join(appData, 'runtimes')].filter(Boolean);
  const model = Boolean(process.env.LOCAL_LLM_BASE_URL)
    || ggufDirs.some(d => dirHas(d, /\.gguf$/i))
    || runtimeDirs.some(d => dirHas(d, /llama-server\.(exe|bin)?$/i))
    || runtimeDirs.some(d => existsSync(d) && readdirSync(d).some(sub => dirHas(join(d, sub), /llama-server\.(exe|bin)?$/i)));

  const searxng = await httpUp(process.env.SEARXNG_BASE_URL || 'http://127.0.0.1:8080');

  let docker = false;
  try { docker = spawnSync('docker', ['info'], { timeout: 5000, stdio: 'ignore' }).status === 0; } catch { /* not installed */ }

  return { 'debug-app': debugApp, model, searxng, docker, node: true };
}

function runSmoke(entry) {
  return new Promise((resolvePromise) => {
    const started = Date.now();
    const child = spawn(process.execPath, [entry.script], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    const onData = (c) => { out += c; };
    child.stdout.on('data', onData);
    child.stderr.on('data', onData);
    const timer = setTimeout(() => { child.kill('SIGTERM'); resolvePromise({ status: 'FAIL', exitCode: null, durationMs: Date.now() - started, output: out, error: `timed out after ${timeoutMs}ms` }); }, timeoutMs);
    child.on('error', (err) => { clearTimeout(timer); resolvePromise({ status: 'FAIL', exitCode: null, durationMs: Date.now() - started, output: out, error: String(err) }); });
    child.on('exit', (code) => { clearTimeout(timer); resolvePromise({ status: code === 0 ? 'PASS' : 'FAIL', exitCode: code, durationMs: Date.now() - started, output: out }); });
  });
}

const selected = only
  ? manifest.filter(e => only.some(o => e.name === o || e.script.includes(o)))
  : manifest;

if (selected.length === 0) {
  console.error(`No smokes matched${only ? ` --only ${only.join(',')}` : ''}. Names: ${manifest.map(e => e.name).join(', ')}`);
  process.exit(2);
}

if (list) {
  for (const e of selected) console.log(`${e.name.padEnd(24)} needs=[${e.needs.join(',')}] ${e.script}${e.notes ? `\n${''.padEnd(24)}   note: ${e.notes}` : ''}`);
  process.exit(0);
}

const prereqs = await detectPrereqs();
console.log('Prerequisites: ' + Object.entries(prereqs).map(([k, v]) => `${k}=${v ? 'yes' : 'NO'}`).join('  '));

const results = [];
let failed = 0;
for (const entry of selected) {
  const missing = entry.needs.filter(n => !prereqs[n]);
  if (missing.length) {
    console.log(`SKIP ${entry.name} (missing: ${missing.join(', ')})`);
    results.push({ name: entry.name, script: entry.script, status: 'SKIP', missing, notes: entry.notes });
    continue;
  }
  process.stdout.write(`RUN  ${entry.name} ... `);
  const r = await runSmoke(entry);
  const lastLine = (r.output || '').trim().split('\n').filter(Boolean).pop() || '';
  if (r.status === 'PASS') {
    console.log(`PASS ${entry.name} (${(r.durationMs / 1000).toFixed(1)}s)${lastLine ? ` — ${lastLine.slice(0, 140)}` : ''}`);
  } else {
    failed++;
    console.log(`FAIL ${entry.name} (${(r.durationMs / 1000).toFixed(1)}s, exit ${r.exitCode ?? r.error})`);
    const tail = (r.output || '').trim().split('\n').slice(-30).join('\n');
    if (tail) console.log(tail.split('\n').map(l => `     ${l}`).join('\n'));
  }
  results.push({ name: entry.name, script: entry.script, status: r.status, exitCode: r.exitCode, durationMs: r.durationMs, missing: [], notes: entry.notes, outputTail: (r.output || '').trim().split('\n').slice(-20).join('\n') });
}

const passed = results.filter(r => r.status === 'PASS').length;
const skipped = results.filter(r => r.status === 'SKIP').length;
console.log(`\n${results.length} smokes: ${passed} PASS, ${failed} FAIL, ${skipped} SKIP`);

if (json) {
  const dir = join(root, 'test-results');
  mkdirSync(dir, { recursive: true });
  const file = join(dir, 'smoke-run.json');
  writeFileSync(file, JSON.stringify({ generatedAt: new Date().toISOString(), prereqs, results }, null, 2));
  console.log(`Wrote ${file}`);
}

process.exit(failed ? 1 : 0);
