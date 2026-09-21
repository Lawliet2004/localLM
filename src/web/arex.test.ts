import { expect, it, vi } from 'vitest';
import { WebSearchEngine, DEFAULT_CONFIG } from './index';
import { extractAtomicClaims } from './verification/claim_extractor';
import { ClaimVerifier } from './verification/claim_verifier';

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

it('post-finish audit flags answer claims absent from collected evidence', () => {
  // Mirrors the worker's verify action: atomic claims from the finish answer,
  // checked deterministically against evidence gathered during the run.
  const claims = extractAtomicClaims(
    'The adapter supports USB-C charging. The warranty lasts five years.',
  );
  const report = new ClaimVerifier().verifyClaimsDeterministic(claims, [{
    id: 'E1',
    claim: 'The adapter supports USB-C charging at up to 60 watts.',
    supportingSources: ['https://example.org/spec'],
    status: 'supported',
    confidence: 1,
  }]);
  expect(report.claims).toHaveLength(2);
  expect(report.supportedCount).toBe(1);
  expect(report.unsupportedCount).toBe(1);
  expect(report.allSupported).toBe(false);
});
