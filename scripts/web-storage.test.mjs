import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SqliteStorage } from './web-storage.mjs';
import { DatabaseSync } from 'node:sqlite';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('persistent cache migrates old rows, bounds weather reports and clears cache without deleting research history', async () => {
  const directory = mkdtempSync(join(tmpdir(), 'locallm-storage-'));
  const path = join(directory, 'cache.sqlite');
  try {
    const old = new DatabaseSync(path);
    old.exec('CREATE TABLE cache(namespace TEXT,key TEXT,value TEXT,expires_at INTEGER,PRIMARY KEY(namespace,key))');
    old.prepare('INSERT INTO cache VALUES(?,?,?,?)').run('search', 'old', '{"value":"世界"}', Date.now() + 10000); old.close();
    let storage = new SqliteStorage(path);
    assert.deepEqual(await storage.get('search', 'old'), { value: '世界' });
    assert.ok(storage.db.prepare('SELECT byte_size FROM cache').get().byte_size > 0);
    for (let index = 0; index < 70; index++) await storage.set('weather-v2', String(index), { temperature: index }, 600);
    assert.equal(storage.db.prepare('SELECT COUNT(*) AS n FROM cache WHERE namespace=?').get('weather-v2').n, 64);
    await storage.set('expired', 'item', { stale: true }, -1);
    assert.equal(await storage.get('expired', 'item'), null);
    storage.save({ id: 'citation-session', sources: [{ url: 'https://example.com' }], documents: [] });
    storage.close(); storage = new SqliteStorage(path);
    assert.equal((await storage.get('weather-v2', '69')).temperature, 69);
    await storage.clear();
    assert.equal(await storage.get('weather-v2', '69'), null);
    assert.equal(storage.load('citation-session').sources[0].url, 'https://example.com');
    storage.close();
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
