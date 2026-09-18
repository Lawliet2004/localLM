import { afterEach, expect, it, vi } from 'vitest';
import { SearXNGProvider } from './searxng_provider';

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
