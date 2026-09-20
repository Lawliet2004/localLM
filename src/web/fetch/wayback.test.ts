import { expect, it } from 'vitest';
import { fetchViaWayback, queryWaybackSnapshot } from './wayback';

it('returns null when the availability API has no snapshot', async () => {
  const fetcher = {
    fetch: async () => ({
      url: 'https://archive.org/wayback/available',
      finalUrl: 'https://archive.org/wayback/available',
      success: true,
      status: 200,
      body: '{"archived_snapshots":{}}',
      durationMs: 0,
    }),
  };
  expect(await queryWaybackSnapshot('https://example.com/gone', fetcher)).toBeNull();
});

it('recovers extracted text with an archive provenance line', async () => {
  const fetcher = {
    fetch: async (url: string) => {
      if (url.includes('archive.org/wayback/available')) {
        return {
          url,
          finalUrl: url,
          success: true,
          status: 200,
          body: JSON.stringify({
            archived_snapshots: {
              closest: {
                available: true,
                url: 'https://web.archive.org/web/20240101000000/https://example.com/gone',
                timestamp: '20240101000000',
              },
            },
          }),
          durationMs: 0,
        };
      }
      return {
        url,
        finalUrl: url,
        success: true,
        status: 200,
        body: '<html><article><h1>Old page</h1><p>Recovered article text that is long enough to keep after the live fetch failed and the archive snapshot was used instead of an empty page.</p></article></html>',
        durationMs: 0,
      };
    },
  };
  const recovered = await fetchViaWayback('https://example.com/gone', fetcher);
  expect(recovered?.archivedAt).toBe('2024-01-01');
  expect(recovered?.text).toContain('Wayback Machine');
  expect(recovered?.text).toContain('Recovered article text');
  expect(recovered?.title).toContain('Archived');
});
