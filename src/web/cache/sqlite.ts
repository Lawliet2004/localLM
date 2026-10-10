/**
 * Cache storage seam for the web search pipeline.
 * InMemoryStorageAdapter is the default; scripts/web-storage.mjs supplies a
 * SQLite-backed implementation for the worker.
 */

export interface CacheEntry<T> {
  value: T;
  expiresAt: number;
}

export interface StorageAdapter {
  claimQuota(table: string, key: string, count: number, limit: number, ttlSeconds?: number): Promise<boolean>;
  get<T>(table: string, key: string): Promise<T | null>;
  set<T>(table: string, key: string, value: T, ttlSeconds?: number): Promise<void>;
  delete(table: string, key: string): Promise<void>;
  clear(table?: string): Promise<void>;
}

export class InMemoryStorageAdapter implements StorageAdapter {
  private stores = new Map<string, Map<string, CacheEntry<any>>>();
  async claimQuota(table: string, key: string, count: number, limit: number, ttlSeconds = 90000): Promise<boolean> {
    if (!Number.isInteger(count) || count < 1) return false;
    const entries = this.getTable(table);
    const entry = entries.get(key);
    const used = entry && entry.expiresAt > Date.now() ? Number(entry.value) : 0;
    if (used + count > limit) return false;
    entries.set(key, { value: used + count, expiresAt: Date.now() + ttlSeconds * 1000 });
    return true;
  }

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

    if (Date.now() >= entry.expiresAt) {
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
