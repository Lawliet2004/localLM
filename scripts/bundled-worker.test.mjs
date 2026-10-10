import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';

test('bundled worker searches room-temperature superconductivity without routing to weather', async () => {
  const server = createServer((_request, response) => {
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify({ results: [], number_of_results: 0 }));
  });
  await new Promise((resolveListen) => server.listen(0, '127.0.0.1', resolveListen));
  const directory = mkdtempSync(join(tmpdir(), 'locallm-worker-test-'));
  try {
    const run = async (question, baseUrl = `http://127.0.0.1:${server.address().port}`) => {
      const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], {
        stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
      });
      child.stdin.end(JSON.stringify({
        question, mode: 'fast', databasePath: join(directory, 'research.sqlite'),
        config: { searxngBaseUrl: baseUrl, cache: { enabled: false }, searchRetries: 0 },
      }));
      let stdout = '';
      let stderr = '';
      for await (const chunk of child.stdout) stdout += chunk;
      for await (const chunk of child.stderr) stderr += chunk;
      const exit = await new Promise((resolveClose) => child.on('close', resolveClose));
      assert.equal(exit, 0, stderr);
      return JSON.parse(stdout);
    };
    const result = await run('Investigate room-temperature superconductivity replication');
    assert.equal(result.route.vertical, 'GENERAL_WEB');
    assert.equal(result.route.requiresWebSearch, true);
    assert.ok(result.trace.queriesGenerated > 0, 'search planner must run');
    assert.doesNotMatch(result.answer, /weather data/i);

    const weather = await run('weather today');
    assert.equal(weather.route.vertical, 'WEATHER');
    assert.ok(weather.trace.queriesGenerated > 0, 'failed weather provider must fall through to search');

    const probe = createServer();
    await new Promise((resolveListen) => probe.listen(0, '127.0.0.1', resolveListen));
    const closedPort = probe.address().port;
    await new Promise((resolveClose) => probe.close(resolveClose));
    const offline = await run('Investigate room-temperature superconductivity replication', `http://127.0.0.1:${closedPort}`);
    assert.ok(offline.trace.queriesGenerated > 0);
    assert.ok(offline.trace.searchFailureCount >= offline.trace.queriesGenerated, 'transport failures must be visible in trace');
    assert.equal(offline.trace.searchResults, 0);
  } finally {
    server.close();
    rmSync(directory, { recursive: true, force: true });
  }
});

test('bundled worker returns URL fetch failures as structured tool errors', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'locallm-worker-fetch-test-'));
  try {
    const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], {
      stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
    });
    child.stdin.end(JSON.stringify({
      action: 'fetch-url',
      url: 'http://127.0.0.1/private',
      databasePath: join(directory, 'research.sqlite'),
      config: { cache: { enabled: false }, fetch: { waybackFallback: false, jsRenderFallback: false } },
    }));
    let stdout = '';
    let stderr = '';
    for await (const chunk of child.stdout) stdout += chunk;
    for await (const chunk of child.stderr) stderr += chunk;
    const exit = await new Promise((resolveClose) => child.on('close', resolveClose));
    assert.equal(exit, 0, stderr);
    const result = JSON.parse(stdout);
    assert.equal(result.isError, true);
    assert.equal(result.failureKind, 'fetch_failed');
    assert.match(result.message, /Non-public address blocked/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
