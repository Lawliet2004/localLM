import { expect, it, vi } from 'vitest';
import { SearchService } from './search_service';

it('does not retry a fixture backend as if it were a transient outage', async () => {
  const search = vi.fn(async () => { throw new Error('SearXNG endpoint returned fixture results, not live web search'); });
  const result = await new SearchService({ search }, { retries: 3 }).executeSearches([
    { query: 'Node.js latest version 2026', purpose: 'test', freshness: 'month' },
  ]);
  expect(search).toHaveBeenCalledTimes(1);
  expect(result.failureCount).toBe(1);
});
