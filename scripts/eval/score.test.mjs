import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { evaluateAssertions, scoreTrajectory, validateTask } from './score.mjs';
import { aggregate, renderMarkdown, summarize } from './report.mjs';

const event = (eventType, payload = {}) => ({ eventType, payload });

test('valid calls, tool errors and denials are counted from session events', () => {
  const metrics = scoreTrajectory([
    event('step_start'),
    event('tool_call', { name: 'read_file' }),
    event('tool_result', { name: 'read_file', decision: 'allowed', result: { schemaVersion: 'locallm.tool-envelope/1', status: 'success' } }),
    event('tool_call', { name: 'edit_file' }),
    event('tool_result', { name: 'edit_file', decision: 'denied', result: { status: 'failure' } }),
    event('step_start'),
    event('turn_end', { status: 'complete', error: null }),
  ], { wallMs: 1200, approvals: [{}, {}], contexts: [{ inputTokens: 900 }, { inputTokens: 1400, estimated: false }] });
  assert.equal(metrics.status, 'complete');
  assert.equal(metrics.rounds, 2);
  assert.equal(metrics.toolCalls, 2);
  assert.deepEqual(metrics.toolCallsByName, { read_file: 1, edit_file: 1 });
  assert.equal(metrics.toolCallValidity, 1);
  assert.equal(metrics.toolErrors, 1);
  assert.equal(metrics.deniedCalls, 1);
  assert.equal(metrics.approvals, 2);
  assert.equal(metrics.peakInputTokens, 1400);
  assert.equal(metrics.outputTokens, null);
  assert.equal(metrics.cacheHitRatio, null);
});

test('timings and slot restore events feed token and cache metrics', () => {
  const metrics = scoreTrajectory([
    event('kv_slot', { action: 'restore', outcome: 'restored', tokens: 1800 }),
    event('timings', { round: 0, cacheN: 1800, promptN: 200, predictedN: 40 }),
    event('timings', { round: 1, cacheN: 2000, promptN: 0, predictedN: 24 }),
    event('turn_end', { status: 'complete' }),
  ]);
  assert.equal(metrics.outputTokens, 64);
  assert.equal(metrics.promptTokensCached, 3800);
  assert.equal(metrics.promptTokensEvaluated, 200);
  assert.equal(metrics.cacheHitRatio, 0.95);
  assert.equal(metrics.kvRestore, 'restored');
});

test('a malformed tool call that ends the turn lowers validity', () => {
  const metrics = scoreTrajectory([
    event('tool_call', { name: 'list_files' }),
    event('turn_end', { status: 'error', error: 'Tool arguments are not valid JSON. The generation may have been truncated; unfinished tool calls are not executed.' }),
  ]);
  assert.equal(metrics.malformedToolCalls, 1);
  assert.equal(metrics.toolCallValidity, 0.5);
});

test('validity is unmeasured, not perfect, when no call was attempted', () => {
  const metrics = scoreTrajectory([event('turn_end', { status: 'complete' })]);
  assert.equal(metrics.toolCallValidity, null);
  assert.equal(metrics.peakInputTokens, null);
});

test('a timeout without turn_end is reported as a timeout', () => {
  assert.equal(scoreTrajectory([], { timedOut: true }).status, 'timeout');
});

test('assertions check answer, files and tool usage', () => {
  const files = { [resolve('/ws', 'src', 'math.js')]: 'return a + b;' };
  const metrics = scoreTrajectory([event('tool_call', { name: 'edit_file' }), event('turn_end', { status: 'complete' })]);
  const results = evaluateAssertions([
    { type: 'answerIncludes', text: 'FIXED' },
    { type: 'answerMatches', pattern: 'wrote the file', flags: 'i', negate: true },
    { type: 'fileIncludes', path: 'src/math.js', text: ['a + b'] },
    { type: 'fileAbsent', path: 'notes/x.md' },
    { type: 'toolCalled', name: 'edit_file' },
    { type: 'toolNotCalled', name: 'create_file' },
    { type: 'maxToolCalls', max: 0 },
    { type: 'status', equals: 'complete' },
  ], { answer: 'I fixed it.', metrics, workspace: '/ws', readFile: path => files[path] ?? null });
  assert.deepEqual(results.map(result => result.pass), [true, true, true, true, true, true, false, true]);
});

test('assertion paths cannot escape the workspace', () => {
  assert.throws(() => evaluateAssertions([{ type: 'fileExists', path: '../outside.txt' }], { answer: '', metrics: {}, workspace: '/ws', readFile: () => null }), /escapes/);
  assert.throws(() => validateTask({ id: 'x', prompt: 'p', assertions: [{ type: 'fileExists', path: '../a' }] }, 'x.json'), /workspace-relative/);
  assert.throws(() => validateTask({ id: 'x', prompt: 'p', assertions: [{ type: 'fileExists', path: 'a', negate: true }] }, 'x.json'), /negate/);
});

test('every shipped task is valid and its fixture exists', () => {
  const dir = 'evals/tasks';
  const files = readdirSync(dir).filter(file => file.endsWith('.json'));
  assert.ok(files.length > 0);
  for (const file of files) {
    const task = validateTask(JSON.parse(readFileSync(join(dir, file), 'utf8')), file);
    assert.equal(`${task.id}.json`, file, 'task id matches its filename');
    if (task.fixture) assert.ok(existsSync(join('evals/fixtures', task.fixture)), `${task.id}: missing fixture`);
  }
});

test('repeats are summarised with sample SD and grouped per configuration', () => {
  assert.deepEqual(summarize([1, 2, 3]), { n: 3, mean: 2, sd: 1, min: 1, max: 3 });
  assert.equal(summarize([5]).sd, null);
  assert.equal(summarize([null, undefined]).n, 0);
  const run = (taskId, passed, wallMs) => ({ model: 'm.gguf', preset: 'standard', taskId, repeat: 1, passed, assertions: [{ type: 'status', pass: passed, detail: 'x' }], metrics: { status: passed ? 'complete' : 'error', wallMs, toolCalls: 1, toolCallValidity: 1, toolErrors: 0, approvals: 0, peakInputTokens: null, outputTokens: null, cacheHitRatio: null } });
  const runs = [run('a', true, 1000), run('a', false, 3000), run('b', true, 2000)];
  const { byTask, byConfig } = aggregate(runs);
  assert.equal(byTask.length, 2);
  assert.equal(byConfig[0].passed, 2);
  assert.equal(byConfig[0].tasks, 2);
  const markdown = renderMarkdown({ startedAt: 'now', seed: 7, repeats: 2, runs, skipped: [{ taskId: 'c', preset: 'minimal', reason: 'r' }] });
  assert.match(markdown, /\| m\.gguf \| standard \| 2 \| 2\/3 \|/);
  assert.match(markdown, /## Skipped/);
  assert.match(markdown, /## Failures/);
});
