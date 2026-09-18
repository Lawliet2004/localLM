import { describe, it, expect } from 'vitest';
import {
  initResearchState, recordRound, followUpQueries, classifyRequirements,
  shouldContinue, rejectCandidate, markContradiction,
} from './research_state';
import { evaluateResearchRetry, isRepeatedQuery } from '../verification/research_retry';
import type { EvidenceClaim } from '../types';

const claim = (text: string, sources = ['S1']): EvidenceClaim => ({
  id: 'CLM-1', claim: text, supportingSources: sources, status: 'supported', confidence: 0.9,
});

describe('iterative research state', () => {
  it('retains all user requirements across a multi-source task', () => {
    const state = initResearchState('Compare Qwen 2.5 Coder and DeepSeek Coder V2 benchmarks');
    expect(state.requirements.length).toBeGreaterThanOrEqual(2);
    expect(state.pendingActions.length).toBe(state.requirements.length);
  });

  it('detects repeated actions and evidence-free rounds', () => {
    const state = initResearchState('What is the latest release?');
    const first = recordRound(state, 'search: release', [claim('Release 2.0 is out.')], { S1: { id: 'S1', title: 't', url: 'https://example.com', domain: 'example.com' } });
    expect(first.newEvidence).toBe(1);
    const repeat = recordRound(state, 'search: release', [claim('Release 2.0 is out.')], { S1: { id: 'S1', title: 't', url: 'https://example.com', domain: 'example.com' } });
    expect(repeat.newEvidence).toBe(0);
    expect(repeat.repeated).toBe(true);
    expect(state.roundsWithoutNewEvidence).toBe(1);
  });

  it('generates gap-driven follow-ups and classifies requirements', () => {
    const state = initResearchState('Compare Alpha and Beta benchmarks');
    const followUps = followUpQueries(state, 2);
    expect(followUps.length).toBeGreaterThan(0);
    const classified = classifyRequirements(state, []);
    expect(classified.every((r) => r.status !== 'answered')).toBe(true);
    expect(classified[0].reason).toBeDefined();
    const answered = classifyRequirements(state, [claim('Alpha benchmarks show 90 points and also mentions Beta benchmarks.')]);
    expect(answered.some((r) => r.status === 'answered')).toBe(true);
  });

  it('stops the loop on stagnation instead of burning budget', () => {
    const state = initResearchState('Obscure topic');
    state.roundsWithoutNewEvidence = 2;
    expect(shouldContinue(state)).toBe(false);
    const decision = evaluateResearchRetry(
      { claims: [{ claim: 'x', status: 'UNSUPPORTED', sources: [] }], allSupported: false, supportedCount: 0, unsupportedCount: 1, conflictingCount: 0 },
      0, 3, ['x'], 2,
    );
    expect(decision.shouldRetry).toBe(false);
  });

  it('refuses to retry an already-tried targeted query', () => {
    expect(isRepeatedQuery('Model X parameters benchmark', ['benchmark parameters model x'])).toBe(true);
    const decision = evaluateResearchRetry(
      { claims: [{ claim: 'Model X parameters unknown', status: 'UNSUPPORTED', sources: [] }], allSupported: false, supportedCount: 0, unsupportedCount: 1, conflictingCount: 0 },
      0, 3, ['model parameters unknown'],
    );
    expect(decision.shouldRetry).toBe(false);
  });

  it('represents contradictory sources accurately', () => {
    const state = initResearchState('What is the context window?');
    markContradiction(state, '128K context', '256K context', ['S1', 'S2']);
    rejectCandidate(state, 'Model Z', 'No supporting evidence found');
    expect(state.contradictions).toHaveLength(1);
    expect(state.rejected[0].reason).toContain('No supporting');
  });

  it('unanswerable questions produce explicit limitations', async () => {
    const { WebSearchEngine } = await import('../index');
    const { MockSearchProvider } = await import('../search/provider');
    const engine = new WebSearchEngine({ searchProvider: new MockSearchProvider({}) });
    const session = await engine.research('What is the release date of NonExistentSuperModel-999B?');
    expect(session.requirements?.length).toBeGreaterThan(0);
    expect(session.requirements?.every((r) => r.status !== 'answered')).toBe(true);
    expect(session.limitations?.join(' ')).toMatch(/REQ-1|unresolved|blocked/i);
  });
});
