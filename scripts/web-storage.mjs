import { DatabaseSync } from 'node:sqlite';

/** Dedicated research database: never expose arbitrary SQL to a model. */
export class SqliteStorage {
  constructor(path) {
    this.db = new DatabaseSync(path);
    this.db.exec(`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
      CREATE TABLE IF NOT EXISTS cache (
        namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
        expires_at INTEGER NOT NULL, byte_size INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(namespace,key));
      CREATE TABLE IF NOT EXISTS research (id TEXT PRIMARY KEY, session TEXT NOT NULL, created_at INTEGER NOT NULL);`);
    this.db.exec('BEGIN IMMEDIATE');
    try {
      if (!this.db.prepare('PRAGMA table_info(cache)').all().some(column => column.name === 'byte_size')) {
        this.db.exec('ALTER TABLE cache ADD COLUMN byte_size INTEGER NOT NULL DEFAULT 0; UPDATE cache SET byte_size=length(CAST(value AS BLOB))');
      }
      this.db.exec('COMMIT');
    } catch (error) { this.db.exec('ROLLBACK'); throw error; }
    this.db.prepare('DELETE FROM cache WHERE expires_at <= ?').run(Date.now());
  }
  async get(table, key) {
    const row = this.db.prepare('SELECT value FROM cache WHERE namespace=? AND key=? AND expires_at>?')
      .get(table, key, Date.now());
    return row ? JSON.parse(row.value) : null;
  }
  async set(table, key, value, ttl = 86400) {
    const serialized = JSON.stringify(value);
    if (Buffer.byteLength(serialized) > 10 * 1024 * 1024) return;
    this.db.exec('BEGIN IMMEDIATE');
    try {
    this.db.prepare('INSERT OR REPLACE INTO cache(namespace,key,value,expires_at,byte_size) VALUES(?,?,?,?,?)')
      .run(table, key, serialized, Date.now() + ttl * 1000, Buffer.byteLength(serialized));
    this.db.prepare('DELETE FROM cache WHERE expires_at <= ?').run(Date.now());
    if (table.startsWith('weather-')) this.db.prepare('DELETE FROM cache WHERE namespace=? AND rowid NOT IN (SELECT rowid FROM cache WHERE namespace=? ORDER BY expires_at DESC,rowid DESC LIMIT 64)').run(table, table);
    this.db.exec('DELETE FROM cache WHERE rowid NOT IN (SELECT rowid FROM cache ORDER BY expires_at DESC,rowid DESC LIMIT 2000)');
    let size = this.db.prepare('SELECT COALESCE(SUM(byte_size),0) AS bytes FROM cache').get().bytes;
    while (size > 64 * 1024 * 1024) {
      const oldest = this.db.prepare('SELECT rowid,byte_size AS bytes FROM cache ORDER BY expires_at,rowid LIMIT 1').get();
      if (!oldest) break;
      this.db.prepare('DELETE FROM cache WHERE rowid=?').run(oldest.rowid); size -= oldest.bytes;
    }
      this.db.exec('COMMIT');
    } catch (error) { this.db.exec('ROLLBACK'); throw error; }
  }
  async delete(table, key) {
    this.db.prepare('DELETE FROM cache WHERE namespace=? AND key=?').run(table, key);
  }
  async clear(table) {
    if (table) this.db.prepare('DELETE FROM cache WHERE namespace=?').run(table);
    else this.db.exec('DELETE FROM cache');
  }
  save(session) {
    this.db.prepare('INSERT OR REPLACE INTO research VALUES(?,?,?)')
      .run(session.id, JSON.stringify(session), Date.now());
  }
  load(id) {
    const row = this.db.prepare('SELECT session FROM research WHERE id=?').get(id);
    if (!row) throw new Error('Unknown research session');
    return JSON.parse(row.session);
  }
  close() { this.db.close(); }
}
