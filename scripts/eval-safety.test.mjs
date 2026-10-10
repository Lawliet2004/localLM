import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve, sep } from 'node:path';
import { tmpdir } from 'node:os';
import test from 'node:test';
import {
  copyRepoSnapshot,
  createEvalRunDirectory,
  requireReadyEvaluationModel,
  restoreWorkspace,
  withWorkspace,
  workspaceForTest,
} from './eval-safety.mjs';

function tempRoot(t) {
  const root = mkdtempSync(join(tmpdir(), 'locallm-eval-safety-'));
  t.after(() => {
    const absolute = resolve(root);
    assert.ok(absolute.startsWith(resolve(tmpdir()) + sep));
    rmSync(absolute, { recursive: true, force: true });
  });
  return root;
}

test('repository snapshots include source and exclude local state and build output', t => {
  const root = tempRoot(t);
  const source = join(root, 'repo');
  const destination = join(root, 'snapshot');
  mkdirSync(join(source, 'src'), { recursive: true });
  mkdirSync(join(source, 'node_modules', 'pkg'), { recursive: true });
  mkdirSync(join(source, 'eval-results'), { recursive: true });
  mkdirSync(join(source, '.local'), { recursive: true });
  mkdirSync(join(source, '.codex', 'secrets'), { recursive: true });
  mkdirSync(join(source, 'target', 'debug'), { recursive: true });
  writeFileSync(join(source, 'src', 'main.rs'), 'source');
  writeFileSync(join(source, '.env.local'), 'secret');
  writeFileSync(join(source, 'state.sqlite'), 'private db');
  writeFileSync(join(source, 'node_modules', 'pkg', 'index.js'), 'dependency');
  writeFileSync(join(source, 'eval-results', 'trace.json'), 'trace');
  writeFileSync(join(source, '.local', 'state.json'), 'local state');
  writeFileSync(join(source, '.codex', 'secrets', 'token.json'), 'private token');
  writeFileSync(join(source, '.npmrc'), '//registry.example/:_authToken=secret');
  writeFileSync(join(source, 'target', 'debug', 'app.exe'), 'build');

  copyRepoSnapshot(source, destination);
  assert.equal(readFileSync(join(destination, 'src', 'main.rs'), 'utf8'), 'source');
  for (const excluded of [
    '.env.local', 'state.sqlite', 'node_modules/pkg/index.js',
    'eval-results/trace.json', '.local/state.json', '.codex/secrets/token.json',
    '.npmrc', 'target/debug/app.exe',
  ]) {
    assert.equal(existsSync(join(destination, excluded)), false, excluded);
  }
});

test('each evaluation run gets a new output directory without overwriting prior results', t => {
  const root = tempRoot(t);
  const first = createEvalRunDirectory(root, 'minicpm', 'run-1');
  writeFileSync(join(first, '02-superconductor-claim.json'), 'historical output');
  const second = createEvalRunDirectory(root, 'minicpm', 'run-2');
  assert.notEqual(first, second);
  assert.equal(readFileSync(join(first, '02-superconductor-claim.json'), 'utf8'), 'historical output');
  assert.throws(() => createEvalRunDirectory(root, 'minicpm', 'run-1'), /already exists|EEXIST/i);
  assert.throws(() => createEvalRunDirectory(root, '../outside', 'run-3'), /path segments/);
});

test('repository snapshots reject a destination inside or above the source', t => {
  const root = tempRoot(t);
  const source = join(root, 'repo');
  mkdirSync(source);
  assert.throws(() => copyRepoSnapshot(source, join(source, 'snapshot')), /outside the source/);
  assert.throws(() => copyRepoSnapshot(source, root), /outside the source/);
});

test('repo tests require a snapshot while scratch tests stay isolated', () => {
  assert.throws(() => workspaceForTest({ id: 6, slug: 'repo', workspace: 'repo' }, 'scratch'), /isolated repository snapshot/);
  assert.equal(
    workspaceForTest({ id: 6, slug: 'repo', workspace: 'repo' }, 'scratch', 'C:/eval/repo-copy'),
    resolve('C:/eval/repo-copy'),
  );
  assert.equal(
    workspaceForTest({ id: 7, slug: 'cache', workspace: 'scratch' }, 'C:/eval/scratch'),
    resolve('C:/eval/scratch/07-cache'),
  );
});

test('evaluation accepts only an already-loaded matching model', () => {
  const model = { filename: 'MiniCPM.gguf', id: 'minicpm' };
  assert.equal(
    requireReadyEvaluationModel({ phase: 'ready', modelPath: 'C:/models/MiniCPM.gguf' }, model),
    model,
  );
  assert.throws(
    () => requireReadyEvaluationModel({ phase: 'idle', modelPath: '' }, model),
    /will not switch models or write saved runtime settings/,
  );
  assert.throws(
    () => requireReadyEvaluationModel({ phase: 'ready', modelPath: 'C:/models/Other.gguf' }, model),
    /will not switch models or write saved runtime settings/,
  );
});

test('workspace changes are restored after success and failure', async () => {
  for (const shouldFail of [false, true]) {
    let current = 'saved-workspace';
    const calls = [];
    const invoke = async (command, args) => {
      calls.push([command, args?.path]);
      if (command === 'get_workspace') return { path: current };
      current = args.path;
    };
    const run = async () => {
      assert.equal(current, 'eval-workspace');
      if (shouldFail) throw new Error('eval failed');
      return 'result';
    };

    if (shouldFail) await assert.rejects(withWorkspace(invoke, 'eval-workspace', run), /eval failed/);
    else assert.equal(await withWorkspace(invoke, 'eval-workspace', run), 'result');
    assert.equal(current, 'saved-workspace');
    assert.deepEqual(calls, [
      ['get_workspace', undefined],
      ['set_workspace', 'eval-workspace'],
      ['set_workspace', 'saved-workspace'],
    ]);
  }
});

test('reconnected evaluations restore the original workspace baseline', async () => {
  let current = 'stale-evaluation-workspace';
  const invoke = async (command, args) => {
    assert.equal(command, 'set_workspace');
    current = args.path;
  };
  await restoreWorkspace(invoke, 'original-user-workspace');
  assert.equal(current, 'original-user-workspace');

  current = 'another-evaluation-workspace';
  await withWorkspace(invoke, 'retry-workspace', async () => {
    assert.equal(current, 'retry-workspace');
  }, 'original-user-workspace');
  assert.equal(current, 'original-user-workspace');
});

test('workspace is restored when selecting the evaluation workspace fails', async () => {
  let current = 'saved-workspace';
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args?.path]);
    if (command === 'get_workspace') return { path: current };
    current = args.path;
    if (args.path === 'eval-workspace') throw new Error('workspace save failed');
  };
  await assert.rejects(withWorkspace(invoke, 'eval-workspace', async () => {}), /workspace save failed/);
  assert.equal(current, 'saved-workspace');
  assert.deepEqual(calls, [
    ['get_workspace', undefined],
    ['set_workspace', 'eval-workspace'],
    ['set_workspace', 'saved-workspace'],
  ]);
});
