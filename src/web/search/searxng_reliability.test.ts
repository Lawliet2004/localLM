import { describe, it, expect, afterEach, vi } from 'vitest';
import { SearXNGProvider } from './searxng_provider';
import { SearchService } from './search_service';
import { ttlForFreshness } from '../cache/search_cache';
import type { SearchResult } from '../types';

afterEach(() => vi.unstubAllGlobals());

function result(id: string, url: string, title = `Title ${id}`): SearchResult {
  return { id, queryId: 'q', title, url, domain: new URL(url).hostname, rank: 1 };
}

describe('SearXNG reliability', () => {
  it('sends pageno and respects the instance engine configuration by default', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response('{"results":[]}'));
    vi.stubGlobal('fetch', fetchMock);
    await new SearXNGProvider().search({ query: 'release', page: 2 });
    const url = new URL(String(fetchMock.mock.calls[0][0]));
    expect(url.searchParams.get('pageno')).toBe('2');
    expect(url.searchParams.has('engines')).toBe(false);
    expect(url.searchParams.has('disabled_engines')).toBe(false);
  });

  it('keeps per-engine diagnostics and distinguishes empty from failure', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      results: [{ title: 'Release', url: 'https://example.com/release' }],
      engines: { duckduckgo: { error: 'timeout' } },
      unresponsive_engines: ['wikipedia'],
    })));
    vi.stubGlobal('fetch', fetchMock);
    const provider = new SearXNGProvider();
    const hits = await provider.search({ query: 'release' });
    expect(hits).toHaveLength(1);
    expect(provider.lastDiagnostics?.engines).toEqual({ duckduckgo: 'timeout', wikipedia: 'unresponsive' });
    expect(provider.lastDiagnostics?.unresponsiveEngines).toEqual(['duckduckgo', 'wikipedia']);
  });

  it('recovers page-two results absent from page one within budget', async () => {
    const byPage = (items: SearchResult[]) => new Response(JSON.stringify({
      results: items.map((r) => ({ title: r.title, url: r.url, content: 'snippet' })),
    }));
    const fetchMock = vi.fn().mockImplementation(async (input: any) => {
      const url = String(input);
      if (url.includes('pageno=2')) return byPage([result('r2', 'https://example.com/recovered')]);
      return byPage([]);
    });
    vi.stubGlobal('fetch', fetchMock);
    const provider = new SearXNGProvider();
    const service = new SearchService(provider, { retries: 0 });
    const pageOne = await service.executeSearches([{ query: 'obscure release', purpose: 'p', freshness: 'any' }], 10);
    expect(pageOne.results).toHaveLength(0);
    expect(pageOne.failureCount).toBe(0);
    const pageTwo = await service.fetchAdditionalPage({ query: 'obscure release', purpose: 'p', freshness: 'any' }, 2, 10);
    expect(pageTwo.results[0].url).toBe('https://example.com/recovered');
    expect(pageTwo.results[0].id).toContain('P2');
  });

  it('keeps healthy results when one query fails and reports the failure', async () => {
    const provider = {
      search: vi.fn().mockImplementation(async (req: any) => {
        if (req.query === 'bad') throw new Error('engine down');
        return [result('ok', 'https://example.com/ok')];
      }),
    };
    const service = new SearchService(provider, { maxConcurrentQueries: 1, retries: 0 });
    const outcome = await service.executeSearches([
      { query: 'good', purpose: 'p', freshness: 'any' },
      { query: 'bad', purpose: 'p', freshness: 'any' },
    ]);
    expect(outcome.results).toHaveLength(1);
    expect(outcome.failureCount).toBe(1);
    expect(outcome.failures[0].query).toBe('bad');
  });

  it('respects Retry-After by not retrying into a cooldown', async () => {
    const provider = {
      search: vi.fn().mockRejectedValue(new Error('SearXNG Retry-After cooldown active')),
    };
    const service = new SearchService(provider, { retries: 3, retryDelayMs: 1 });
    const outcome = await service.executeSearches([{ query: 'q', purpose: 'p', freshness: 'any' }]);
    expect(provider.search).toHaveBeenCalledTimes(1);
    expect(outcome.failureCount).toBe(1);
  });

  it('does not retry a permanent SearXNG configuration error', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response('', { status: 403 }));
    vi.stubGlobal('fetch', fetchMock);
    const service = new SearchService(new SearXNGProvider(), { retries: 3, retryDelayMs: 0 });
    const outcome = await service.executeSearches([{ query: 'release', purpose: 'p', freshness: 'any' }]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(outcome.failureCount).toBe(1);
    expect(outcome.failures[0].error).toContain('search.formats');
  });

  it('uses freshness-aware cache TTLs', () => {
    expect(ttlForFreshness('realtime', 86400)).toBe(300);
    expect(ttlForFreshness('day', 86400)).toBe(3600);
    expect(ttlForFreshness('any', 3600)).toBe(3600);
  });

  it('aligns client timeout with the upstream 8s request budget', async () => {
    const fetchMock = vi.fn().mockImplementation((_input: any, init: any) => new Promise((_resolve, reject) => {
      init.signal.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })));
    }));
    vi.stubGlobal('fetch', fetchMock);
    const provider = new SearXNGProvider('http://127.0.0.1:8080', 50);
    await expect(provider.search({ query: 'slow' })).rejects.toThrow('timed out after 50ms');
  }, 10000);

  it('verifies engine availability instead of assuming it', async () => {
    const fetchMock = vi.fn().mockImplementation(async (input: any) => {
      const url = String(input);
      if (url.includes('engines=duckduckgo')) return Response.json({ results: [{ url: 'https://example.com', title: 'SearXNG' }] });
      return new Response('', { status: 500 });
    });
    vi.stubGlobal('fetch', fetchMock);
    const health = await new SearXNGProvider('http://127.0.0.1:8080', 8000, ['duckduckgo', 'broken']).checkEngineHealth();
    expect(health.working).toEqual(['duckduckgo']);
    expect(health.failing['broken']).toContain('HTTP 500');
  });
});
