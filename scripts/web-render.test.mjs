import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderPage, findEdgeBinary } from './web-render.mjs';

test('findEdgeBinary picks the first existing candidate', () => {
  const found = findEdgeBinary(['missing.exe', 'present.exe'], (p) => p === 'present.exe');
  assert.equal(found, 'present.exe');
  assert.equal(findEdgeBinary(['nope'], () => false), null);
});

test('renderPage refuses non-public and non-HTTP targets before spawning', async () => {
  // The browser resolves DNS itself, so the SSRF check must run first —
  // a missing binary would prove the check was skipped by spawning anyway.
  assert.equal(await renderPage('http://127.0.0.1:8080/internal', { edgePath: 'nonexistent.exe' }), null);
  assert.equal(await renderPage('file:///C:/Windows/win.ini', { edgePath: 'nonexistent.exe' }), null);
  assert.equal(await renderPage('not a url', { edgePath: 'nonexistent.exe' }), null);
});

test('renderPage returns null when the browser binary is missing', async () => {
  assert.equal(
    await renderPage('https://example.org/', { edgePath: 'C:\\definitely-not-edge\\msedge.exe', timeoutMs: 5000 }),
    null,
  );
});
