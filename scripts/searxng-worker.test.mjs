import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { resolve } from 'node:path';

// Exercise the shipped worker over real HTTP, without an external search service.
const requests = [];
const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://localhost');
  requests.push(url);
  res.setHeader('Content-Type', 'application/json');
  if (url.pathname === '/forbidden/search') { res.writeHead(403).end('{}'); return; }
  if (url.pathname === '/html/search') { res.end('<html>Not a JSON API</html>'); return; }
  if (url.pathname === '/blocked/search') {
    res.end(JSON.stringify({ results: [], unresponsive_engines: [['duckduckgo', 'CAPTCHA']] })); return;
  }
  if (url.pathname !== '/proxy/search') { res.writeHead(404).end('{}'); return; }
  res.end(JSON.stringify({
    results: [{ url: 'https://docs.searxng.org/dev/search_api.html', title: 'SearXNG Search API', content: 'JSON search API documentation', engine: 'fixture' }],
    answers: [{ answer: url.searchParams.get('q'), url: 'https://docs.searxng.org/dev/search_api.html' }],
  }));
});
let base;
before(async () => {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  base = `http://127.0.0.1:${server.address().port}`;
});
after(() => new Promise(resolve => server.close(resolve)));

async function worker(input) {
  const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', error = '';
  child.stdout.setEncoding('utf8').on('data', chunk => { output += chunk; });
  child.stderr.setEncoding('utf8').on('data', chunk => { error += chunk; });
  const done = new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve); });
  child.stdin.end(JSON.stringify({ ...input, config: { searchRetries: 0, searxngTimeoutMs: 2000, fetch: { timeoutSeconds: 2, jsRenderFallback: false, waybackFallback: false }, ...input.config } }));
  const code = await done;
  assert.equal(code, 0, error);
  return JSON.parse(output);
}

async function workerRaw(input) {
  const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], { stdio: ['pipe', 'pipe', 'pipe'] });
  let output = '', error = '';
  child.stdout.setEncoding('utf8').on('data', chunk => { output += chunk; });
  child.stderr.setEncoding('utf8').on('data', chunk => { error += chunk; });
  const done = new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve); });
  child.stdin.end(JSON.stringify(input));
  await done;
  return { output, error };
}

test('bundled worker checks JSON search through a reverse proxy prefix', async () => {
  const status = await worker({ action: 'health', config: { searxngBaseUrl: `${base}/proxy` } });
  assert.equal(status.search, 'healthy');
  assert.equal(status.searxngDiagnostics.httpStatus, 200);
  assert.equal(requests.at(-1).searchParams.get('format'), 'json');
});

test('bundled worker fails over between instance URLs and keeps per-query metadata', async () => {
  const result = await worker({ action: 'search', query: ['C++ release', 'SearXNG API'], config: { searxngBaseUrl: `${base}/forbidden; ${base}/proxy/search` } });
  assert.equal(result.results.length, 1);
  assert.equal(result.rawCount, 2);
  assert.equal(result.failureCount, 0);
  assert.deepEqual(new Set(result.meta.answers.map(answer => answer.answer)), new Set(['C++ release', 'SearXNG API']));
});

test('bundled worker reports JSON setup problems instead of healthy homepages', async () => {
  for (const prefix of ['forbidden', 'html']) {
    const status = await worker({ action: 'health', config: { searxngBaseUrl: `${base}/${prefix}` } });
    assert.equal(status.search, 'error');
    assert.match(status.searchError, /search.formats/);
  }
});

test('bundled worker distinguishes upstream engine failure from empty search', async () => {
  const result = await worker({ action: 'search', query: ['release'], config: { searxngBaseUrl: `${base}/blocked` } });
  assert.equal(result.failureCount, 1);
  assert.match(result.failures[0].error, /duckduckgo: CAPTCHA/);
});

test('research emits \x1e progress events on stderr and honors language filter', async () => {
  const { output, error } = await workerRaw({
    question: 'SearXNG search API documentation',
    full: true,
    mode: 'fast',
    language: 'en',
    databasePath: ':memory:',
    config: {
      searxngBaseUrl: `${base}/proxy`,
      searchRetries: 0,
      searxngTimeoutMs: 2000,
      fetch: { timeoutSeconds: 2, jsRenderFallback: false, waybackFallback: false, domainLearning: false },
      cache: { enabled: false },
    },
  });
  const stages = error.split('\n')
    .filter((line) => line.startsWith('\x1e'))
    .map((line) => JSON.parse(line.slice(1)).stage);
  assert.deepEqual(stages, ['search'], 'the worker performs only the requested search');
  const result = JSON.parse(output);
  assert.equal(result.route.requiresWebSearch, true);
  assert.deepEqual(result.queries.map(query => query.query), ['SearXNG search API documentation']);
  assert.equal(result.documents[0].metadata.extractionMethod, 'snippet_only');
  assert.equal(requests.at(-1).searchParams.get('language'), 'en');
});
