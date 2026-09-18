import { afterEach, describe, expect, it, vi } from 'vitest';
import { WebSearchEngine } from './index';
import { DEFAULT_CONFIG } from './config/defaults';
import { HttpFetcher } from './fetch/http_fetcher';
import { QueryPlanner } from './planning/query_planner';
import { ClaimVerifier } from './verification/claim_verifier';
import { EvidenceExtractor } from './evidence/extractor';
import { AnswerGenerator } from './generation/answer_generator';
import { buildBudgetedContext } from './context/context_builder';
import type { SearchResult } from './types';

const initialFact = 'The latest release includes faster startup and improved diagnostics.';
const retryFact = 'The benchmark measured throughput of 1234 tokens per second on the reference hardware.';
const result = (id: string, snippet: string): SearchResult => ({
  id, queryId: 'q', title: `Release ${id}`, url: `https://${id}.example.com/release`,
  domain: `${id}.example.com`, snippet, rank: 1,
});

function fixture(results: SearchResult[] = []) {
  vi.spyOn(QueryPlanner.prototype, 'planQueries').mockResolvedValue([
    { query: 'latest release', purpose: 'Find release', freshness: 'any' },
  ]);
  const fetcher = new HttpFetcher();
  vi.spyOn(fetcher, 'fetch').mockImplementation(async (url) => ({
    url, finalUrl: url, success: false, durationMs: 0, error: 'Use supplied snippet',
  }));
  const search = vi.fn().mockResolvedValue(results);
  const engine = new WebSearchEngine({
    searchProvider: { search }, fetcher,
    config: {
      cache: { ...DEFAULT_CONFIG.cache, enabled: false },
      verification: { enabled: true, maxResearchRetries: 0 },
    },
  });
  return { engine, search, fetcher };
}

afterEach(() => vi.restoreAllMocks());

describe('research session integration regressions', () => {
  it('rejects empty model verification and retains every claim for auditing', async () => {
    const verifier = new ClaimVerifier({generate: async () => ({text:'{"claims":[]}'})});
    const report = await verifier.verifyClaims(['An invented model achieved 99 percent accuracy.'], []);
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

  it('does not cite fabricated internal URLs for structured time answers', async () => {
    const { engine } = fixture();
    const session = await engine.research('What time is it in Tokyo?');
    expect(session.route.vertical).toBe('TIME');
    expect(session.answer).toContain('Tokyo'.toUpperCase());
    expect(session.answer).not.toContain('.internal');
    expect(Object.values(session.sources).every((source) => !source.url.includes('.internal'))).toBe(true);
  });

  it.each(['evidence', 'token usage'] as const)('returns the final retry %s', async (field) => {
    const { engine, search } = fixture([result('initial', initialFact)]);
    engine.config.verification.maxResearchRetries = 1;
    search.mockResolvedValueOnce([result('initial', initialFact)])
      .mockResolvedValueOnce([result('retry', retryFact)]);
    vi.spyOn(EvidenceExtractor.prototype, 'extractFacts').mockImplementation(async (_query, chunk, sourceId) => [{
      statement: chunk.url.includes('retry') ? retryFact : initialFact,
      confidence: 0.9, sourceId, chunkId: chunk.id,
    }]);
    vi.spyOn(ClaimVerifier.prototype, 'verifyClaims').mockResolvedValueOnce({
      claims: [{ claim: retryFact, status: 'UNSUPPORTED', sources: [] }],
      allSupported: false, supportedCount: 0, unsupportedCount: 1, conflictingCount: 0,
    }).mockResolvedValue({
      claims: [], allSupported: true, supportedCount: 2, unsupportedCount: 0, conflictingCount: 0,
    });
    const generation = vi.spyOn(AnswerGenerator.prototype, 'generateAnswer');
    const session = await engine.research('Find the latest release benchmarks');
    expect(search).toHaveBeenCalledTimes(2);
    expect(generation).toHaveBeenCalledTimes(2);
    const finalGeneration = generation.mock.calls[generation.mock.calls.length - 1]!;
    expect(finalGeneration[2].some((claim) => claim.claim === retryFact)).toBe(true);
    if (field === 'evidence') {
      expect(session.evidence).toEqual(finalGeneration[2]);
    } else {
      // The session must describe the prompt that actually produced its answer.
      const {allocation: finalBudget} = buildBudgetedContext(session.question, session.sources, finalGeneration[2], engine.config.context, new Date().toISOString().split('T')[0]);
      expect(session.tokenUsage).toEqual(finalBudget.tokenStats);
      expect(session.trace.finalEvidenceTokens).toBe(finalGeneration[0].evidenceTokens);
    }
  });
});
