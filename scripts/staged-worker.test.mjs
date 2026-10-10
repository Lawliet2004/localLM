import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';

async function listen(server) {
  await new Promise(resolveListen => server.listen(0, '127.0.0.1', resolveListen));
  return `http://127.0.0.1:${server.address().port}`;
}

test('bundled worker runs the staged local-model research path', async () => {
  const prompts = [];
  const searches = [];
  const model = createServer(async (request, response) => {
    response.setHeader('Content-Type', 'application/json');
    if (request.url === '/v1/chat/completions/input_tokens') { response.end('{"input_tokens":3}'); return; }
    if (request.url === '/tokenize') {
      response.end(JSON.stringify({ tokens: [1, 2, 3] }));
      return;
    }
    let input = '';
    for await (const chunk of request) input += chunk;
    const body = JSON.parse(input);
    const system = body.messages[0].content;
    prompts.push({ system, temperature: body.temperature });
    const content = system.includes('web search queries') ? 'Node.js latest version 2026'
      : system.includes('fact extractor') ? 'S1.1'
        : 'Node.js 24.1.0 is current. [S1]';
    response.end(JSON.stringify({ choices: [{ message: { content }, finish_reason: 'stop' }] }));
  });
  const search = createServer((request, response) => {
    const params = new URL(request.url, 'http://localhost').searchParams;
    searches.push({ query: params.get('q'), timeRange: params.get('time_range') });
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({
      results: [{ title: 'Node release', url: 'https://release.example/version', content: 'Node.js 24.1.0 is current.' }],
      number_of_results: 1,
    }));
  });
  const directory = mkdtempSync(join(tmpdir(), 'locallm-staged-worker-'));
  try {
    const [modelUrl, searchUrl] = await Promise.all([listen(model), listen(search)]);
    const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], {
      stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
    });
    child.stdin.end(JSON.stringify({
      question: 'What is the latest Node.js version?', mode: 'fast', fetchPages: false,
      databasePath: join(directory, 'research.sqlite'),
      localModel: { baseUrl: `${modelUrl}/v1`, modelName: 'test-model', maxInputTokens: 6000 },
      config: { searxngBaseUrl: searchUrl, cache: { enabled: false }, searchRetries: 0 },
    }));
    let stdout = '';
    let stderr = '';
    for await (const chunk of child.stdout) stdout += chunk;
    for await (const chunk of child.stderr) stderr += chunk;
    const exit = await new Promise(resolveClose => child.on('close', resolveClose));
    assert.equal(exit, 0, stderr);
    const result = JSON.parse(stdout);
    assert.deepEqual(searches, [{ query: 'Node.js latest version 2026', timeRange: 'month' },
      { query: 'Node.js latest version primary source verify 2026', timeRange: 'month' }]);
    assert.equal(prompts.length, 2); // the exact excerpt needs no synthesis; a repeated URL needs no extraction
    assert.deepEqual(prompts.map(prompt => prompt.temperature), [0, 0]);
    assert.match(result.answer, /Node\.js 24\.1\.0 is current/);
    assert.match(result.answer, /https:\/\/release\.example\/version/);
    assert.equal(result.trace.queriesGenerated, 2);
    assert.equal(result.evidence.length, 1);
    assert.equal(result.trace.extractionDiagnostics[0].outcome, 'accepted');
    assert.equal(result.evidence[0].assessment, 'snippet_only');
    assert.equal(result.verification.scope, 'extracted_evidence');
    assert.match(result.answer, /could not independently verify/i);
  } finally {
    model.close();
    search.close();
    rmSync(directory, { recursive: true, force: true });
  }
});

test('bundled AREX search shortens a question and excludes irrelevant hits', async () => {
  const searched = [];
  const search = createServer((request, response) => {
    searched.push(new URL(request.url, 'http://localhost').searchParams.get('q'));
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({ results: [
      { title: 'Football scores', url: 'https://sports.example/scores', content: 'League results from yesterday.' },
      { title: 'Node.js release', url: 'https://nodejs.org/releases', content: 'Node.js version 24.1.0 release notes.' },
    ], number_of_results: 2 }));
  });
  const directory = mkdtempSync(join(tmpdir(), 'locallm-arex-search-'));
  try {
    const searchUrl = await listen(search);
    const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], {
      stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
    });
    child.stdin.end(JSON.stringify({
      action: 'search', query: ['What is the latest Node.js version?'],
      databasePath: join(directory, 'research.sqlite'),
      config: { searxngBaseUrl: searchUrl, cache: { enabled: false }, searchRetries: 0 },
    }));
    let stdout = '';
    let stderr = '';
    for await (const chunk of child.stdout) stdout += chunk;
    for await (const chunk of child.stderr) stderr += chunk;
    const exit = await new Promise(resolveClose => child.on('close', resolveClose));
    assert.equal(exit, 0, stderr);
    const result = JSON.parse(stdout);
    assert.deepEqual(searched, ['Node.js latest version 2026']);
    assert.deepEqual(result.queriesUsed, searched);
    assert.equal(result.results.length, 1);
    assert.equal(result.results[0].url, 'https://nodejs.org/releases');
  } finally {
    search.close();
    rmSync(directory, { recursive: true, force: true });
  }
});
