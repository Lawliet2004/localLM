#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const question = process.argv.slice(2).filter((a) => !a.startsWith('--')).join(' ') || 'What is the newest stable release of React?';

const stubResults = [
  { url: 'https://react.dev/blog/2024/12/05/react-19', title: 'React 19 Release Notes', content: 'React 19 is now stable on npm. React 19 adds Actions, useActionState, useOptimistic and server components support. Upgrade with npm install react@19 react-dom@19.', engine: 'stub' },
  { url: 'https://react.dev/blog/2024/12/05/react-19?utm_source=smoke&utm_medium=test', title: 'React 19 Release Notes', content: 'Syndicated duplicate of the React 19 announcement used to exercise deduplication.', engine: 'stub' },
  { url: 'https://nodejs.org/en/blog/release/v22.12.0', title: 'Node.js v22.12.0 LTS Release', content: 'Node.js v22.12.0 LTS ships npm 10.9 and V8 12.4. Off-topic result used to exercise ranking.', engine: 'stub' },
  { url: 'https://www.python.org/downloads/release/python-3130/', title: 'Python 3.13.0 Release', content: 'Python 3.13 introduces an experimental free-threaded build and an improved REPL.', engine: 'stub' },
  { url: 'http://169.254.169.254/latest/meta-data/', title: 'Cloud metadata probe', content: 'SSRF probe the fetcher must refuse to retrieve.', engine: 'stub' },
];

const server = createServer((req, res) => {
  const u = new URL(req.url || '/', 'http://127.0.0.1');
  if (u.pathname !== '/search') { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify({ query: u.searchParams.get('q') || '', number_of_results: stubResults.length, results: stubResults }));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const stubBase = `http://127.0.0.1:${server.address().port}`;

mkdirSync(resolve(root, '.local'), { recursive: true });
const child = spawn(process.execPath, [resolve(root, 'src-tauri/resources/web/worker.mjs')], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, cwd: root });
const payload = {
  question, mode: 'normal', trace: true, full: true,
  databasePath: resolve(root, '.local/phase1-smoke.sqlite'),
  config: { searxngBaseUrl: stubBase, cache: { enabled: false } },
  ...(process.env.LOCAL_LLM_BASE_URL ? { localModel: { baseUrl: process.env.LOCAL_LLM_BASE_URL, modelName: process.env.LOCAL_LLM_MODEL } } : {}),
};
child.stdin.end(JSON.stringify(payload));
let out = '';
let err = '';
for await (const c of child.stdout) out += c;
for await (const c of child.stderr) err += c;
const code = await new Promise((r) => child.on('close', r));
server.close();

if (code !== 0) {
  console.error(`worker failed (exit ${code}): ${err.slice(-2000)}`);
  process.exitCode = 1;
  process.exit();
}

const s = JSON.parse(out);
const t = s.trace;
const checks = [
  ['router sends general query to web search', s.route?.requiresWebSearch === true],
  [`planner emits 1-4 queries (got ${s.queries?.length})`, s.queries?.length >= 1 && s.queries?.length <= 4],
  [`dedup shrinks raw results (raw ${t.searchResults} -> unique ${t.uniqueResults})`, t.searchResults >= t.uniqueResults && t.uniqueResults >= 2],
  ['SSRF probe never fetched', !(s.documents || []).some((d) => String(d.url).includes('169.254'))],
  [`pages fetched (ok ${t.pagesFetched}, fail ${t.fetchFailures})`, t.pagesFetched >= 1],
  [`chunks created (${t.chunksCreated})`, t.chunksCreated >= 1],
  [`evidence budget respected (${t.finalEvidenceTokens} <= 2500)`, t.finalEvidenceTokens <= 2500],
  [`prompt budget respected (${t.finalPromptTokens} <= 6000)`, t.finalPromptTokens <= 6000],
  ['no raw HTML leaks into evidence', !(s.evidence || []).some((c) => /<\s*html|<\s*div/i.test(c.claim))],
  ['every cited [S#] resolves to a real source URL', (s.answer?.match(/S\d+/g) || []).every((id) => s.sources?.[id]?.url?.startsWith('https://'))],
];

const ratio = t.finalEvidenceTokens > 0 ? (t.extractedTokens / t.finalEvidenceTokens).toFixed(1) : 'N/A';
console.log(`--- PHASE 1 SMOKE (stub SearXNG JSON at loopback, real HTTPS page fetches) ---`);
console.log(`Question: ${s.question}`);
console.log(`LLM path: ${process.env.LOCAL_LLM_BASE_URL ? 'local model (narrow calls + deterministic fallback)' : 'deterministic fallback (set LOCAL_LLM_BASE_URL to exercise the model path)'}`);
console.log(`Routing: ${s.route?.vertical} (freshness ${s.route?.freshness})`);
console.log(`Queries (${s.queries?.length}): ${(s.queries || []).map((q) => `"${q.query}"`).join(' | ')}`);
console.log(`Raw results: ${t.searchResults} / Unique: ${t.uniqueResults} / Pages fetched: ${t.pagesFetched} (fail ${t.fetchFailures})`);
console.log(`Extracted tokens: ${t.extractedTokens} / Chunks: ${t.chunksCreated} / Evidence claims: ${t.evidenceClaims}`);
console.log(`Evidence tokens to LLM: ${t.finalEvidenceTokens} / Final prompt tokens: ${t.finalPromptTokens} / Compression: ${ratio}x`);
for (const [label, ok] of checks) console.log(`${ok ? 'PASS' : 'FAIL'}  ${label}`);
console.log(`--- ANSWER ---\n${s.answer}`);
if (err.trim()) console.log(`--- WORKER TRACE ---\n${err.trim()}`);
if (checks.some(([, ok]) => !ok)) process.exitCode = 1;
