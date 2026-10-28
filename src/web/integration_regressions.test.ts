import { afterEach, describe, expect, it, vi } from 'vitest';
import { WebSearchEngine } from './index';
import { DEFAULT_CONFIG } from './config/defaults';
import { HttpFetcher } from './fetch/http_fetcher';
import { ClaimVerifier } from './verification/claim_verifier';
import type { SearchResult } from './types';

const initialFact = 'The latest release includes faster startup and improved diagnostics.';
const retryFact = 'The benchmark measured throughput of 1234 tokens per second on the reference hardware.';
const result = (id: string, snippet: string): SearchResult => ({
  id, queryId: 'q', title: `Release ${id}`, url: `https://${id}.example.com/release`,
  domain: `${id}.example.com`, snippet, rank: 1,
});

function fixture(results: SearchResult[] = []) {
  const fetcher = new HttpFetcher();
  vi.spyOn(fetcher, 'fetch').mockImplementation(async (url) => ({
    url, finalUrl: url, success: false, durationMs: 0, error: 'Use supplied snippet',
  }));
  const search = vi.fn().mockResolvedValue(results);
  const engine = new WebSearchEngine({
    searchProvider: { search }, fetcher,
    config: {
      cache: { ...DEFAULT_CONFIG.cache, enabled: false },
    },
  });
  return { engine, search, fetcher };
}

afterEach(() => vi.restoreAllMocks());

describe('research session integration regressions', () => {
  it('retains unsupported claims for auditing', async () => {
    const verifier = new ClaimVerifier();
    const report = verifier.verifyClaimsDeterministic(['An invented model achieved 99 percent accuracy.'], []);
    expect(report.allSupported).toBe(false);
    expect(report.unsupportedCount).toBe(1);
  });
  it('does not treat negation as support', () => {
    const report = new ClaimVerifier().verifyClaimsDeterministic(['The model does not support image input.'], [{id:'c',claim:'The model does support image input.',supportingSources:['S1'],status:'supported',confidence:1}]);
    expect(report.allSupported).toBe(false);
  });
  it('assigns unique document and chunk identities to concurrent fetches', async () => {
    const { engine } = fixture([result('first', initialFact), result('second', retryFact)]);
    const session = await engine.research('Find the latest release benchmarks');
    expect(session.documents).toHaveLength(2);
    expect(new Set(session.documents.map((doc) => doc.id)).size).toBe(2);
    expect(new Set(session.chunks.map((chunk) => chunk.id)).size).toBe(session.chunks.length);
  });

  it('does not search or fetch when the engine is disabled', async () => {
    const { engine, search, fetcher } = fixture();
    engine.config.enabled = false;
    // Either an explicit disabled error or a local answer is acceptable.
    await engine.research('Find the latest release benchmarks').catch(() => undefined);
    expect(search).not.toHaveBeenCalled();
    expect(fetcher.fetch).not.toHaveBeenCalled();
  });


});
