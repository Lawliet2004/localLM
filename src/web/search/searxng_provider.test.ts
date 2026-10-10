import { afterEach, expect, it, vi } from 'vitest';
import { documentsFromSearchMeta, metaForResults, parseSearxngBaseUrls, SearXNGProvider, normalizeSearxMeta } from './searxng_provider';
import { WebSearchEngine } from '../index';
import { InMemoryStorageAdapter } from '../cache/sqlite';

it('counts all-unresponsive engines as a failure but retains partial results', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ results: [], unresponsive_engines: [['yep', 'timeout'], ['duckduckgo', 'blocked']] }))));
  const provider = new SearXNGProvider('http://localhost:8080', 1000, ['yep', 'duckduckgo']);
  await expect(provider.search({ query: 'adapter' })).rejects.toThrow(/yep.*timeout.*duckduckgo.*blocked/);
  const outcome = await new WebSearchEngine({ searchProvider: provider, config: { searchRetries: 0 } }).searchQueries(['adapter']);
  expect(outcome.failureCount).toBeGreaterThan(0);
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ results: [{ title: 'Adapter', url: 'https://spec.example/adapter', content: 'Adapter charging' }], unresponsive_engines: [['yep', 'timeout']] }))));
  expect(await provider.search({ query: 'adapter' })).toHaveLength(1);
});

it('engine health reports unresponsive engine reasons', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ results: [], unresponsive_engines: [['yep', 'timeout']] }))));
  const health = await new SearXNGProvider('http://localhost:8080', 1000, ['yep']).checkEngineHealth();
  expect(health).toEqual({ working: [], failing: { yep: 'timeout' } });
});

it('persists Retry-After across providers sharing a storage', async () => {
  const storage = new InMemoryStorageAdapter();
  const fetchMock = vi.fn(async () => new Response('', { status: 429, headers: { 'Retry-After': '120' } }));
  vi.stubGlobal('fetch', fetchMock);
  // Constructor storage is introduced with the fix; assignment keeps the regression executable before it.
  const first = new SearXNGProvider();
  const second = new SearXNGProvider();
  Object.assign(first, { storage }); Object.assign(second, { storage });
  await expect(first.search({ query: 'one' })).rejects.toThrow('429');
  await expect(second.search({ query: 'two' })).rejects.toThrow('Retry-After');
  expect(fetchMock).toHaveBeenCalledTimes(1);
});

it('preserves base path prefixes for search and engine health', async () => {
  const fetchMock = vi.fn(async (_input: string) => new Response('{"results":[]}'));
  vi.stubGlobal('fetch', fetchMock);
  for (const base of ['http://localhost:8080', 'http://localhost:8080/searxng/']) {
    const provider = new SearXNGProvider(base, 1000, ['yep']);
    await provider.search({ query: 'adapter' }); await provider.checkEngineHealth();
    const expected = base.includes('searxng') ? '/searxng/search' : '/search';
    expect(fetchMock.mock.calls.slice(-2).every(call => new URL(String(call[0])).pathname === expected)).toBe(true);
  }
});

afterEach(() => vi.unstubAllGlobals());

it('keeps the connection cause and endpoint in a failed search diagnostic', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(Object.assign(new TypeError('fetch failed'), {
    cause: Object.assign(new Error('connect ECONNREFUSED'), { code: 'ECONNREFUSED' }),
  })));
  const provider = new SearXNGProvider('http://127.0.0.1:8080');
  await expect(provider.search({ query: 'GPT-6 Astra benchmarks' })).rejects.toThrow(/127\.0\.0\.1:8080.*ECONNREFUSED/);
  expect(provider.lastDiagnostics?.error).toContain('ECONNREFUSED');
});

it('leaves time for a second endpoint when the first one times out', async () => {
  vi.stubGlobal('fetch', vi.fn(async (input: string, init: RequestInit) => {
    if (input.includes(':8080')) return new Promise<Response>((_resolve, reject) => {
      init.signal?.addEventListener('abort', () => reject(Object.assign(new Error('aborted'), { name: 'AbortError' })));
    });
    return new Response(JSON.stringify({ results: [{ title: 'Model benchmarks', url: 'https://example.com/benchmarks', content: 'Scores.' }] }));
  }));
  const results = await new SearXNGProvider('http://127.0.0.1:8080,http://127.0.0.1:8081', 200)
    .search({ query: 'model benchmarks' });
  expect(results[0].url).toBe('https://example.com/benchmarks');
});

it('never follows search-service redirects to an untrusted destination', async () => {
  const fetchMock = vi.fn().mockResolvedValue(new Response('{"results":[]}'));
  vi.stubGlobal('fetch', fetchMock);
  await new SearXNGProvider().search({ query: 'current release' });
  expect(fetchMock.mock.calls[0][1].redirect).toBe('error');
});

it('honors Retry-After before making another search request', async () => {
  const fetchMock = vi.fn().mockResolvedValue(new Response('', { status: 429, headers: { 'Retry-After': '120' } }));
  vi.stubGlobal('fetch', fetchMock);
  const provider = new SearXNGProvider();
  await expect(provider.search({ query: 'release' })).rejects.toThrow('429');
  await expect(provider.search({ query: 'release again' })).rejects.toThrow('Retry-After');
  expect(fetchMock).toHaveBeenCalledTimes(1);
});

it('fails over to the next SearXNG instance inside the shared timeout budget', async () => {
  const fetchMock = vi.fn().mockImplementation(async (input: any) => {
    const url = String(input);
    if (url.includes('8080')) return new Response('', { status: 503 });
    return new Response(JSON.stringify({
      results: [{ title: 'Hit', url: 'https://example.com/hit', content: 'ok' }],
      answers: ['React 19'],
    }));
  });
  vi.stubGlobal('fetch', fetchMock);
  const provider = new SearXNGProvider('http://127.0.0.1:8080,http://127.0.0.1:8081', 8000);
  const hits = await provider.search({ query: 'react' });
  expect(hits[0].url).toBe('https://example.com/hit');
  expect(provider.lastMeta?.answers[0].answer).toBe('React 19');
  expect(metaForResults(hits)?.answers[0].answer).toBe('React 19');
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

it('normalizes infoboxes and turns them into evidence documents', () => {
  const meta = normalizeSearxMeta({
    answers: [{ answer: 'Paris is the capital of France', url: 'https://en.wikipedia.org/wiki/Paris' }],
    infoboxes: [{ infobox: 'Paris', content: 'Capital of France', urls: [{ url: 'https://en.wikipedia.org/wiki/Paris' }] }],
    suggestions: ['paris weather'],
    corrections: ['Pariis'],
  });
  expect(meta.infoboxes[0].title).toBe('Paris');
  const docs = documentsFromSearchMeta(meta);
  expect(docs.some((d) => d.text.includes('capital of France'))).toBe(true);
  expect(docs.some((d) => d.metadata?.extractionMethod === 'searxng_infobox')).toBe(true);
});

it('parses comma and semicolon instance lists', () => {
  expect(parseSearxngBaseUrls('http://127.0.0.1:8080; http://127.0.0.1:8081')).toEqual([
    'http://127.0.0.1:8080',
    'http://127.0.0.1:8081',
  ]);
});

it('rejects a fixture server that returns HTTP 200 with invented search results', async () => {
  const fetchMock = vi.fn().mockImplementation(async () => new Response(JSON.stringify({
    number_of_results: 1,
    results: [{ title: 'Example result', url: 'https://example.com/', content: 'Example Domain', engine: 'fixture' }],
  })));
  vi.stubGlobal('fetch', fetchMock);
  const provider = new SearXNGProvider();
  await expect(provider.search({ query: 'Node.js latest version 2026' })).rejects.toThrow(/fixture/i);
  expect(provider.lastDiagnostics?.error).toMatch(/fixture/i);
  const health = await provider.checkEngineHealth();
  expect(health.working).toEqual([]);
  expect(Object.values(health.failing)).toEqual(expect.arrayContaining([expect.stringMatching(/fixture/i)]));
  const engineHealth = await new WebSearchEngine({ searchProvider: provider }).getHealthStatus();
  expect(engineHealth.search).toBe('fixture');
});
