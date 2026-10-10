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
  async claimQuota(table, key, count, limit, ttl = 90000) {
    if (!Number.isInteger(count) || count < 1 || count > limit) return false;
    const now = Date.now();
    // One upsert is atomic across worker processes, including expiry reset.
    const result = this.db.prepare(`INSERT INTO cache(namespace,key,value,expires_at) VALUES(?,?,?,?)
      ON CONFLICT(namespace,key) DO UPDATE SET
        value = CASE WHEN cache.expires_at <= ? THEN ? ELSE CAST(cache.value AS INTEGER) + ? END,
        expires_at = CASE WHEN cache.expires_at <= ? THEN excluded.expires_at ELSE cache.expires_at END
      WHERE cache.expires_at <= ? OR CAST(cache.value AS INTEGER) + ? <= ?`)
      .run(table, key, String(count), now + ttl * 1000, now, count, count, now, now, count, limit);
    return result.changes === 1;
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
