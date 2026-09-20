/**
 * Search Cache: Caches SERP results per query with intent-dependent TTL.
 * Freshness-aware: breaking/realtime windows get short TTLs, stable
 * documentation gets long ones, so stale news never serves as current fact.
 */

import type { SearchResult, SearchMeta, FreshnessWindow } from '../types';
import { InMemoryStorageAdapter, type StorageAdapter } from './sqlite';

const FRESHNESS_TTL_SECONDS: Record<FreshnessWindow, number> = {
  realtime: 300,
  day: 3600,
  week: 21600,
  month: 86400,
  year: 86400 * 7,
  any: 86400,
};

export function ttlForFreshness(freshness: FreshnessWindow, fallback: number): number {
  return Math.min(fallback, FRESHNESS_TTL_SECONDS[freshness] ?? fallback);
}

export class SearchCache {
  constructor(private storage: StorageAdapter = new InMemoryStorageAdapter()) {}

  private makeKey(provider: string, query: string, freshness: FreshnessWindow = 'any'): string {
    return `${provider}::${query.toLowerCase().trim()}::${freshness}`;
  }

  async get(provider: string, query: string, freshness: FreshnessWindow = 'any'): Promise<SearchResult[] | null> {
    const key = this.makeKey(provider, query, freshness);
    return this.storage.get<SearchResult[]>('search_cache', key);
  }

  async set(
    provider: string,
    query: string,
    results: SearchResult[],
    freshness: FreshnessWindow = 'any',
    ttlSeconds: number = 86400
  ): Promise<void> {
    const key = this.makeKey(provider, query, freshness);
    await this.storage.set('search_cache', key, results, ttlSeconds);
  }

  /** SearXNG answers/infoboxes/corrections/suggestions cached beside the organic results. */
  async getMeta(provider: string, query: string, freshness: FreshnessWindow = 'any'): Promise<SearchMeta | null> {
    return this.storage.get<SearchMeta>('search_cache', `${this.makeKey(provider, query, freshness)}::meta`);
  }

  async setMeta(
    provider: string,
    query: string,
    meta: SearchMeta,
    freshness: FreshnessWindow = 'any',
    ttlSeconds: number = 86400
  ): Promise<void> {
    if (meta.answers.length || meta.infoboxes.length || meta.corrections.length || meta.suggestions.length) {
      await this.storage.set('search_cache', `${this.makeKey(provider, query, freshness)}::meta`, meta, ttlSeconds);
    }
  }
}

