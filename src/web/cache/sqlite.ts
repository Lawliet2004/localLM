/**
 * Cache Storage & SQLite Schema: SQLite table definitions and unified storage adapter
 * for local persistence of search results, extracted documents, and embeddings.
 */

export const SQLITE_CACHE_SCHEMA = `
CREATE TABLE IF NOT EXISTS search_cache (
    cache_key TEXT PRIMARY KEY,
    query TEXT NOT NULL,
    provider TEXT NOT NULL,
    freshness TEXT,
    results_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS document_cache (
    url TEXT PRIMARY KEY,
    canonical_url TEXT,
    title TEXT,
    author TEXT,
    published_at TEXT,
    text TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    chunk_id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT,
    heading_path TEXT,
    text TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    published_at TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS embedding_cache (
    cache_key TEXT PRIMARY KEY,
    model_id TEXT NOT NULL,
    embedding_json TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS queries (
    query_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    query_text TEXT NOT NULL,
    purpose TEXT,
    freshness TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id TEXT PRIMARY KEY,
    question TEXT NOT NULL,
    route_json TEXT NOT NULL,
    token_usage_json TEXT NOT NULL,
    answer TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sources (
    source_id TEXT PRIMARY KEY,
    session_id TEXT,
    url TEXT NOT NULL,
    domain TEXT NOT NULL,
    title TEXT NOT NULL,
    published_at TEXT,
    authority_score REAL
);

CREATE TABLE IF NOT EXISTS fetch_metadata (
    url TEXT PRIMARY KEY,
    status_code INTEGER NOT NULL,
    mime_type TEXT,
    content_length INTEGER,
    duration_ms INTEGER,
    etag TEXT,
    last_modified TEXT,
    fetched_at INTEGER NOT NULL
);
`;

export interface CacheEntry<T> {
  value: T;
  expiresAt: number;
}

export interface StorageAdapter {
  get<T>(table: string, key: string): Promise<T | null>;
  set<T>(table: string, key: string, value: T, ttlSeconds?: number): Promise<void>;
  delete(table: string, key: string): Promise<void>;
  clear(table?: string): Promise<void>;
}

export class InMemoryStorageAdapter implements StorageAdapter {
  private stores = new Map<string, Map<string, CacheEntry<any>>>();

  private getTable(table: string): Map<string, CacheEntry<any>> {
    let t = this.stores.get(table);
    if (!t) {
      t = new Map();
      this.stores.set(table, t);
    }
    return t;
  }

  async get<T>(table: string, key: string): Promise<T | null> {
    const t = this.getTable(table);
    const entry = t.get(key);
    if (!entry) return null;

    if (Date.now() > entry.expiresAt) {
      t.delete(key);
      return null;
    }

    return entry.value as T;
  }

  async set<T>(table: string, key: string, value: T, ttlSeconds: number = 86400): Promise<void> {
    const t = this.getTable(table);
    t.set(key, {
      value,
      expiresAt: Date.now() + ttlSeconds * 1000,
    });
  }

  async delete(table: string, key: string): Promise<void> {
    this.getTable(table).delete(key);
  }

  async clear(table?: string): Promise<void> {
    if (table) {
      this.stores.get(table)?.clear();
    } else {
      this.stores.clear();
    }
  }
}
