import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { publishIfChanged } from './build-web-worker.mjs';

test('publishIfChanged writes once then leaves identical output untouched', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'locallm-worker-test-'));
  const source = join(dir, 'worker.mjs');
  const destination = join(dir, 'nested', 'worker.mjs');
  await writeFile(source, 'bundled output');
  assert.equal(await publishIfChanged(source, destination), 'written');
  const first = (await stat(destination)).mtimeMs;
  await new Promise(resolve => setTimeout(resolve, 20));
  assert.equal(await publishIfChanged(source, destination), 'unchanged');
  assert.equal((await stat(destination)).mtimeMs, first);
  await writeFile(source, 'bundled output v2');
  assert.equal(await publishIfChanged(source, destination), 'written');
});
