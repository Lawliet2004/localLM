import { afterEach, expect, it, vi } from 'vitest';
import { GoogleSearchProvider } from './google_provider';
afterEach(() => vi.unstubAllGlobals());

it('never exposes the API key from HTTP bodies or transport errors', async () => {
  const key = 'secret-key-123';
  for (const failure of ['body', 'transport']) {
    vi.stubGlobal('fetch', async () => {
      if (failure === 'transport') throw new Error(`fetch failed for https://googleapis.com/?key=${key}`);
      return new Response(`invalid key ${key}`, { status: 403 });
    });
    const error = await new GoogleSearchProvider(key, 'cx').search({ query: 'adapter' }).catch(value => value);
    expect(String(error)).not.toContain(key);
  }
});

it('maps freshness and page to Google request parameters', async () => {
  const fetchMock = vi.fn(async (_input: string) => new Response('{"items":[]}'));
  vi.stubGlobal('fetch', fetchMock);
  await new GoogleSearchProvider('key', 'cx').search({ query: 'adapter', freshness: 'week', page: 3, maxResults: 10 });
  const url = new URL(String(fetchMock.mock.calls[0][0]));
  expect(url.searchParams.get('dateRestrict')).toBe('w1');
  expect(url.searchParams.get('start')).toBe('21');
});
