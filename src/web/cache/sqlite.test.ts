import { describe, it, expect, vi } from 'vitest';
import { InMemoryStorageAdapter } from './sqlite';
import { SearchCache } from './search_cache';
import type { SearchResult } from '../types';

describe('Storage & Cache Expiration', () => {
  it('stores and retrieves cached items', async () => {
    const storage = new InMemoryStorageAdapter();
    await storage.set('test_table', 'key1', { foo: 'bar' }, 10);

    const val = await storage.get<{ foo: string }>('test_table', 'key1');
    expect(val).toEqual({ foo: 'bar' });
  });

  it('expires entries past TTL', async () => {
    const storage = new InMemoryStorageAdapter();
    // Set 0 second TTL (immediate expire)
    await storage.set('test_table', 'key2', 'expired_val', -1);

    const val = await storage.get<string>('test_table', 'key2');
    expect(val).toBeNull();
  });

  it('expires entries exactly at the TTL boundary', async () => {
    const now = Date.now();
    const clock = vi.spyOn(Date, 'now').mockReturnValue(now);
    try {
      const storage = new InMemoryStorageAdapter();
      await storage.set('cache', 'item', 'value', 1);
      clock.mockReturnValue(now + 1000);
      expect(await storage.get('cache', 'item')).toBeNull();
    } finally { clock.mockRestore(); }
  });

  it('caches search results by provider, query, and freshness', async () => {
    const storage = new InMemoryStorageAdapter();
    const cache = new SearchCache(storage);

    const results: SearchResult[] = [
      { id: '1', queryId: 'Q', title: 'Title', url: 'https://example.com', domain: 'example.com', rank: 1 },
    ];

    await cache.set('searxng', 'latest coding models', results, 'month', 3600);
    const cached = await cache.get('searxng', 'latest coding models', 'month');

    expect(cached).toHaveLength(1);
    expect(cached?.[0].title).toBe('Title');
  });
});
