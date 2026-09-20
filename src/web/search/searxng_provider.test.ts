import { afterEach, expect, it, vi } from 'vitest';
import { documentsFromSearchMeta, metaForResults, parseSearxngBaseUrls, SearXNGProvider, normalizeSearxMeta } from './searxng_provider';

afterEach(() => vi.unstubAllGlobals());

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
