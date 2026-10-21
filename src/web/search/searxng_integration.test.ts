import { afterEach, expect, it, vi } from 'vitest';
import { WebSearchEngine, DEFAULT_CONFIG } from '../index';

afterEach(() => vi.unstubAllGlobals());

it('preserves the primary provider failure when direct search succeeds through fallback', async () => {
  vi.stubGlobal('fetch', vi.fn(async (input) => String(input).includes('googleapis.com')
    ? Response.json({ items: [{ title: 'Release', link: 'https://example.com/release', snippet: 'Release details' }] })
    : new Response('', { status: 403 })));
  const result = await new WebSearchEngine({ config: { googleApiKey: 'fixture', googleCxId: 'fixture', searchRetries: 0 } }).searchQueries(['release']);
  expect(result.results).toHaveLength(1);
  expect(result.failureCount).toBe(1);
  expect(result.failures[0].error).toContain('search.formats');
});

it('keeps failure diagnostics in research sessions when SearXNG is misconfigured', async () => {
  const fetchMock = vi.fn().mockResolvedValue(new Response('', { status: 403 }));
  vi.stubGlobal('fetch', fetchMock);
  const session = await new WebSearchEngine({ config: {
    searchRetries: 3,
    queries: { fast: 1, normal: 1, deep: 1 },
    cache: { ...DEFAULT_CONFIG.cache, enabled: false },
  } }).research('Find the latest software release', { mode: 'fast' });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(session.trace.searchFailureCount).toBe(1);
  expect(session.trace.queryFailures?.[0].error).toContain('search.formats');
  expect(session.trace.searxngDiagnostics?.httpStatus).toBe(403);
  expect(session.documents).toEqual([]);
});

it('does not paginate beyond an exhausted research query budget', async () => {
  const fetchMock = vi.fn().mockImplementation(async () => Response.json({ results: [] }));
  vi.stubGlobal('fetch', fetchMock);
  const session = await new WebSearchEngine({ config: {
    searchRetries: 0, enablePagination: true,
    queries: { fast: 1, normal: 1, deep: 1 },
    cache: { ...DEFAULT_CONFIG.cache, enabled: false },
  } }).research('Find the latest software release', { mode: 'fast' });
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(session.queries).toHaveLength(1);
  expect(session.trace.searchFailureCount).toBe(0);
});

it('keeps a structured SearXNG answer without calling a secondary provider', async () => {
  const fetchMock = vi.fn().mockImplementation(async () => Response.json({ results: [], answers: ['A structured answer'] }));
  vi.stubGlobal('fetch', fetchMock);
  const result = await new WebSearchEngine({ config: { googleApiKey: 'fixture', googleCxId: 'fixture' } }).searchQueries(['fact']);
  expect(fetchMock).toHaveBeenCalledTimes(1);
  expect(result.meta.answers[0].answer).toBe('A structured answer');
});
