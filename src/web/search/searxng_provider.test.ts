import { afterEach, expect, it, vi } from 'vitest';
import { documentsFromSearchMeta, metaForResults, parseSearxngBaseUrls, SearXNGProvider, normalizeSearxMeta } from './searxng_provider';

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

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

it('preserves reverse-proxy prefixes and accepts pasted search endpoints', async () => {
  const fetchMock = vi.fn().mockResolvedValue(Response.json({ results: [] }));
  vi.stubGlobal('fetch', fetchMock);
  await new SearXNGProvider('https://example.com/tools/searx/search/').search({
    query: 'C++ site:example.com & release', language: 'en', category: 'news', freshness: 'week', page: 3,
  });
  const url = new URL(fetchMock.mock.calls[0][0]);
  expect(url.pathname).toBe('/tools/searx/search');
  expect(url.searchParams.get('q')).toBe('C++ site:example.com & release');
  expect(url.searchParams.get('language')).toBe('en');
  expect(url.searchParams.get('categories')).toBe('news');
  expect(url.searchParams.get('time_range')).toBe('month');
  expect(url.searchParams.get('pageno')).toBe('3');
});

it.each(['file:///tmp/search', 'localhost:8080', 'https://example.com?q=test', 'https://example.com#x', 'https://user:pass@example.com'])('rejects invalid service URL %s before fetching', async (url) => {
  const fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
  await expect(new SearXNGProvider(url).search({ query: 'release' })).rejects.toThrow('base URL');
  expect(fetchMock).not.toHaveBeenCalled();
});

it('applies engine overrides without re-enabling disabled engines', async () => {
  const fetchMock = vi.fn().mockResolvedValue(Response.json({ results: [] }));
  vi.stubGlobal('fetch', fetchMock);
  await new SearXNGProvider('http://localhost:8080', 1000, ['duckduckgo', 'wikipedia'], ['wikipedia']).search({ query: 'release' });
  const url = new URL(fetchMock.mock.calls[0][0]);
  expect(url.searchParams.get('engines')).toBe('duckduckgo');
  expect(url.searchParams.get('disabled_engines')).toBe('wikipedia');
});

it('distinguishes a valid empty search from engine failures in HTTP 200', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(Response.json({ results: [] }))
    .mockResolvedValueOnce(Response.json({ results: [], unresponsive_engines: [['duckduckgo', 'CAPTCHA'], ['wikipedia', 'timeout']] })));
  const provider = new SearXNGProvider();
  await expect(provider.search({ query: 'no matches' })).resolves.toEqual([]);
  await expect(provider.search({ query: 'blocked' })).rejects.toThrow('engine failures');
  expect(provider.lastDiagnostics?.engines).toEqual({ duckduckgo: 'CAPTCHA', wikipedia: 'timeout' });
});

it('retains good results when other engines fail and skips malformed entries', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({
    results: [null, {}, { url: 'javascript:alert(1)' }, { url: '/relative' }, { title: 'Release', url: 'https://example.com/release', content: 'Stable release' }],
    unresponsive_engines: [['duckduckgo', 'timeout']],
  })));
  const provider = new SearXNGProvider();
  const results = await provider.search({ query: 'release', maxResults: 1 });
  expect(results).toHaveLength(1);
  expect(results[0].url).toBe('https://example.com/release');
  expect(provider.lastDiagnostics?.engines).toEqual({ duckduckgo: 'timeout' });
});

it.each(['<html>Search homepage</html>', '{}', '{"error":"Invalid engines"}', 'null'])('rejects malformed API responses: %s', async (body) => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(body)));
  await expect(new SearXNGProvider().search({ query: 'release' })).rejects.toThrow(/JSON|invalid search response/);
});

it('explains how to enable JSON after a forbidden response', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 403 })));
  const provider = new SearXNGProvider();
  await expect(provider.search({ query: 'release' })).rejects.toThrow('search.formats');
  expect(provider.lastDiagnostics?.httpStatus).toBe(403);
});

it('keeps metadata attached to concurrent queries and clears stale metadata on failure', async () => {
  vi.stubGlobal('fetch', vi.fn(async (input) => {
    const query = new URL(String(input)).searchParams.get('q');
    if (query === 'fail') throw new Error('offline');
    return Response.json({ results: [], answers: [query] });
  }));
  const provider = new SearXNGProvider();
  const [first, second] = await Promise.all([provider.search({ query: 'first' }), provider.search({ query: 'second' })]);
  expect(metaForResults(first)?.answers).toEqual([{ answer: 'first' }]);
  expect(metaForResults(second)?.answers).toEqual([{ answer: 'second' }]);
  await expect(provider.search({ query: 'fail' })).rejects.toThrow('offline');
  expect(provider.lastMeta).toBeNull();
  expect(provider.lastDiagnostics?.error).toContain('offline');
});

it('reserves timeout budget for a second instance when the first hangs', async () => {
  vi.useFakeTimers();
  const fetchMock = vi.fn((input, init) => {
    if (String(input).includes('8081')) return Promise.resolve(Response.json({ results: [{ url: 'https://example.com/hit' }] }));
    return new Promise<Response>((_resolve, reject) => init.signal.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))));
  });
  vi.stubGlobal('fetch', fetchMock);
  const pending = new SearXNGProvider('http://localhost:8080;http://localhost:8081', 1000).search({ query: 'release' });
  await vi.advanceTimersByTimeAsync(500);
  expect((await pending)[0].url).toBe('https://example.com/hit');
  expect(fetchMock).toHaveBeenCalledTimes(2);
});

it.each(['2', new Date(Date.now() + 2000).toUTCString()])('honors Retry-After per instance: %s', async (retryAfter) => {
  vi.useFakeTimers();
  const fetchMock = vi.fn((input) => Promise.resolve(String(input).includes('8080')
    ? new Response('', { status: 429, headers: { 'Retry-After': retryAfter } })
    : Response.json({ results: [] })));
  vi.stubGlobal('fetch', fetchMock);
  const provider = new SearXNGProvider('http://localhost:8080;http://localhost:8081', 1000);
  await provider.search({ query: 'release' });
  await provider.search({ query: 'second' });
  expect(fetchMock.mock.calls.map(([url]) => new URL(String(url)).port)).toEqual(['8080', '8081', '8081']);
  await vi.advanceTimersByTimeAsync(2001);
  await provider.search({ query: 'third' });
  expect(fetchMock).toHaveBeenCalledTimes(5);
});

it('reports degraded health when usable results coexist with engine failures', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({
    results: [{ url: 'https://example.com' }], unresponsive_engines: [['wikipedia', 'timeout']],
  })));
  expect(await new SearXNGProvider().checkHealth()).toMatchObject({
    search: 'degraded', searxngDiagnostics: { engines: { wikipedia: 'timeout' } },
  });
});
