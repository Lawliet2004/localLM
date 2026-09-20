import { expect, it, vi } from 'vitest';
import { WebSearchEngine, DEFAULT_CONFIG } from './index';

it('AREX search returns sources without invoking a nested research model', async () => {
  const generate = vi.fn();
  const search = vi.fn(async ({ query }) => [{ id: query, queryId: query, title: query,
    url: `https://example.org/${query}`, domain: 'example.org', rank: 1, snippet: 'source excerpt' }]);
  const engine = new WebSearchEngine({
    config: { cache: { ...DEFAULT_CONFIG.cache, enabled: false } },
    searchProvider: { search }, llmProvider: { generate },
  });
  const result = await engine.searchQueries(['one', 'two']);
  expect(search).toHaveBeenCalledTimes(2);
  expect(result.results).toHaveLength(2);
  expect(generate).not.toHaveBeenCalled();
  await expect(engine.searchQueries([])).rejects.toThrow();
  await expect(engine.searchQueries(['x'.repeat(8001)])).rejects.toThrow();
});
