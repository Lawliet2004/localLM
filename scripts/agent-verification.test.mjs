import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { resolve } from 'node:path';
import { test } from 'node:test';

async function verify(answer, evidence) {
  const child = spawn(process.execPath, [resolve('src-tauri/resources/web/worker.mjs')], {
    stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
  });
  const closed = new Promise(resolveClose => child.on('close', resolveClose));
  child.stdin.end(JSON.stringify({ action:'verify', databasePath:':memory:', answer, evidence }));
  let stdout = '';
  let stderr = '';
  await Promise.all([
    (async () => { for await (const chunk of child.stdout) stdout += chunk; })(),
    (async () => { for await (const chunk of child.stderr) stderr += chunk; })(),
  ]);
  assert.equal(await closed, 0, stderr);
  return JSON.parse(stdout);
}

test('citation formatting does not turn an exact source fact into a verification gap', async () => {
  const claim = 'Node.js 24.0.0 was released on May 6, 2025.';
  const report = await verify(`${claim} [Release](https://nodejs.org/release)`, [
    { url:'https://nodejs.org/release', claim },
  ]);
  assert.equal(report.supportedCount, 1);
  assert.equal(report.unsupportedCount, 0);
});

test('contradictory sources trigger review even when the answer copies one exactly', async () => {
  const report = await verify('The adapter supplies 60 watts.', [
    { url:'https://manufacturer.example/spec', claim:'The adapter supplies 60 watts.' },
    { url:'https://testing.example/spec', claim:'The adapter supplies 90 watts.' },
  ]);
  assert.equal(report.allSupported, false);
  assert.equal(report.conflictingCount, 1);
});

test('different products with different numbers are separate facts', async () => {
  const report = await verify('The Alpha adapter supplies 60 watts.', [
    { url:'https://manufacturer.example/alpha', claim:'The Alpha adapter supplies 60 watts.' },
    { url:'https://testing.example/beta', claim:'The Beta adapter supplies 90 watts.' },
  ]);
  assert.equal(report.allSupported, true);
  assert.equal(report.conflictingCount, 0);
});
