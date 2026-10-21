#!/usr/bin/env node
/**
 * Repeatable fixture benchmark for the bundled web-research worker.
 *
 * Spawns a local SearXNG-compatible fixture and runs a fixed question set
 * through the real worker binary. Page fetches to public URLs are expected to
 * fail offline (SSRF keeps loopback out of fetched pages), so the fixture
 * exercises routing, planning, search, ranking, evidence, caching, and citation
 * assembly — not live extraction. Set LOCAL_LLM_BASE_URL/LOCAL_LLM_MODEL to add
 * a real model round-trip and token counts.
 *
 *   node scripts/web-benchmark.mjs [--json out.json]
 */
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const QUESTIONS = [
  { id: 'static-fact', mode: 'fast', question: 'What year was the telephone invented?', expectSearch: false },
  { id: 'tech-docs', mode: 'normal', question: 'Python 3.13 new features documentation', expectSearch: true },
  { id: 'fresh-info', mode: 'normal', question: 'What is the latest stable llama.cpp release?', expectSearch: true },
  { id: 'multi-source', mode: 'deep', question: 'Compare Qwen2.5 Coder and DeepSeek Coder coding benchmarks', expectSearch: true },
];

const FIXTURE_RESULTS = [
  { url: 'https://docs.python.org/3.13/whatsnew/3.13.html', title: 'What’s New In Python 3.13', content: 'Python 3.13 adds free-threaded execution and improved error messages.', engine: 'fixture' },
  { url: 'https://github.com/ggml-org/llama.cpp/releases', title: 'llama.cpp releases', content: 'Latest llama.cpp release notes and binaries.', engine: 'fixture' },
  { url: 'https://huggingface.co/Qwen/Qwen2.5-Coder-7B', title: 'Qwen2.5-Coder-7B model card', content: 'Qwen2.5-Coder-7B scores 88.4 on HumanEval.', engine: 'fixture' },
  { url: 'https://github.com/deepseek-ai/DeepSeek-Coder-V2', title: 'DeepSeek-Coder-V2 repository', content: 'DeepSeek-Coder-V2 achieves 81.1 on HumanEval.', engine: 'fixture' },
  { url: 'https://en.wikipedia.org/wiki/History_of_the_telephone', title: 'Telephone history', content: 'The telephone was patented in 1876.', engine: 'fixture' },
];

const fixture = createServer((req, res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname !== '/search') { res.writeHead(404).end('{}'); return; }
  res.setHeader('Content-Type', 'application/json');
  const q = (url.searchParams.get('q') || '').toLowerCase();
  const scored = FIXTURE_RESULTS
    .map((r) => ({ r, hits: (r.title + ' ' + r.content).toLowerCase().split(/\W+/).filter((w) => w.length > 4 && q.includes(w)).length }))
    .sort((a, b) => b.hits - a.hits)
    .slice(0, 6)
    .map((x, i) => ({ ...x.r, score: 1 - i * 0.05 }));
  res.end(JSON.stringify({ results: scored, answers: q.includes('python') ? [{ answer: 'Python 3.13 was released October 2024.' }] : [] }));
});
await new Promise((r) => fixture.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${fixture.address().port}`;

const workerPath = resolve('src-tauri/resources/web/worker.mjs');
const dbDir = mkdtempSync(join(tmpdir(), 'locallm-bench-'));
const databasePath = join(dbDir, 'web-research.sqlite');
const localModel = process.env.LOCAL_LLM_BASE_URL
  ? { baseUrl: process.env.LOCAL_LLM_BASE_URL, modelName: process.env.LOCAL_LLM_MODEL || 'local-model' }
  : undefined;

function runWorker(input) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(process.execPath, [workerPath], { stdio: ['pipe', 'pipe', 'pipe'] });
    let out = '', err = '';
    child.stdout.setEncoding('utf8').on('data', (c) => { out += c; });
    child.stderr.setEncoding('utf8').on('data', (c) => { err += c; });
    child.on('error', reject);
    child.on('close', (code) => {
      const progress = err.split('\n').filter((l) => l.startsWith('\x1e')).map((l) => { try { return JSON.parse(l.slice(1)); } catch { return null; } }).filter(Boolean);
      resolvePromise({ code, out, err, progress });
    });
    child.stdin.end(JSON.stringify({
      ...input,
      databasePath,
      config: {
        searxngBaseUrl: base,
        searchRetries: 0,
        searxngTimeoutMs: 3000,
        fetch: { timeoutSeconds: 2, jsRenderFallback: false, waybackFallback: false, domainLearning: false },
        verification: { enabled: false, maxResearchRetries: 0 },
        ...input.config,
      },
      ...(localModel ? { localModel } : {}),
    }));
  });
}

const citationRe = /\[(?:S|E)\d+\]|https?:\/\/[^\s)\]]+/g;
const rows = [];
for (const item of QUESTIONS) {
  for (const pass of [1, 2]) {
    const started = Date.now();
    const { code, out, err, progress } = await runWorker({ question: item.question, mode: item.mode });
    const wallMs = Date.now() - started;
    if (code !== 0) { rows.push({ id: item.id, pass, error: err.slice(0, 300) }); continue; }
    const session = JSON.parse(out);
    const trace = session.trace || {};
    const citations = (session.answer || '').match(citationRe) || [];
    rows.push({
      id: item.id,
      pass,
      mode: item.mode,
      wallMs,
      totalMs: trace.totalLatencyMs,
      searchMs: trace.latencies?.search ?? null,
      planMs: trace.latencies?.planning ?? null,
      queries: trace.queriesGenerated ?? session.queries?.length ?? 0,
      results: trace.uniqueResults ?? 0,
      pagesFetched: trace.pagesFetched ?? 0,
      fetchFailures: trace.fetchFailures ?? 0,
      evidenceClaims: trace.evidenceClaims ?? session.evidence?.length ?? 0,
      evidenceTokens: trace.finalEvidenceTokens ?? null,
      promptTokens: trace.finalPromptTokens ?? null,
      sources: session.sources ? Object.keys(session.sources).length : 0,
      citations: citations.length,
      stages: progress.map((p) => p.stage).join('>'),
      searched: (trace.queriesGenerated ?? 0) > 0 || (session.queries?.length ?? 0) > 0,
    });
  }
}
fixture.close();

const json = process.argv.includes('--json') ? process.argv[process.argv.indexOf('--json') + 1] : null;
if (json) writeFileSync(json, JSON.stringify({ generatedAt: new Date().toISOString(), localModel: Boolean(localModel), rows }, null, 2));

const fmt = (v) => (v === null || v === undefined ? '—' : String(v));
console.log('\nquestion      pass  wall(ms)  total(ms)  search(ms)  queries  results  pages  evidence  citations  cached-speedup');
for (const r of rows) {
  if (r.error) { console.log(`${r.id.padEnd(14)}${r.pass}      ERROR ${r.error}`); continue; }
  console.log(`${r.id.padEnd(14)}${r.pass}      ${fmt(r.wallMs).padEnd(8)}  ${fmt(r.totalMs).padEnd(9)}  ${fmt(r.searchMs).padEnd(10)}  ${fmt(r.queries).padEnd(7)}  ${fmt(r.results).padEnd(8)}  ${fmt(r.pagesFetched).padEnd(6)}  ${fmt(r.evidenceClaims).padEnd(9)}  ${fmt(r.citations).padEnd(10)}`);
}
const byId = Object.groupBy ? Object.groupBy(rows.filter((r) => !r.error), (r) => r.id) : {};
console.log('\ncache speedup (pass1 → pass2 wall):');
for (const [id, rs] of Object.entries(byId)) {
  if (rs.length === 2) console.log(`  ${id}: ${rs[0].wallMs}ms → ${rs[1].wallMs}ms`);
}
console.log(`\nFixture: ${base}  Database: ${databasePath}`);
console.log('Note: page fetches to public fixture URLs fail offline by design (SSRF); evidence comes from snippets/answers.');
