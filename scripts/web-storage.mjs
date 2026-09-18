import { DatabaseSync } from 'node:sqlite';

/** Dedicated research database: never expose arbitrary SQL to a model. */
export class SqliteStorage {
  constructor(path) {
    this.db = new DatabaseSync(path);
    this.db.exec(`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
      CREATE TABLE IF NOT EXISTS cache (
        namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
        expires_at INTEGER NOT NULL, PRIMARY KEY(namespace,key));
      CREATE TABLE IF NOT EXISTS research (id TEXT PRIMARY KEY, session TEXT NOT NULL, created_at INTEGER NOT NULL);`);
    this.db.prepare('DELETE FROM cache WHERE expires_at <= ?').run(Date.now());
  }
  async get(table, key) {
    const row = this.db.prepare('SELECT value FROM cache WHERE namespace=? AND key=? AND expires_at>?')
      .get(table, key, Date.now());
    return row ? JSON.parse(row.value) : null;
  }
  async set(table, key, value, ttl = 86400) {
    this.db.prepare('INSERT OR REPLACE INTO cache VALUES(?,?,?,?)')
      .run(table, key, JSON.stringify(value), Date.now() + ttl * 1000);
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
