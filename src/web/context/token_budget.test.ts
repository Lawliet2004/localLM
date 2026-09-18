import { describe, it, expect } from 'vitest';
import { allocateEvidenceBudget } from './token_budget';
import type { EvidenceClaim } from '../types';
import { DEFAULT_CONFIG } from '../config/defaults';

describe('Token Budget Manager', () => {
  it('never exceeds configured budget and drops complete evidence units', () => {
    const claims: EvidenceClaim[] = [];
    for (let i = 1; i <= 50; i++) {
      claims.push({
        id: `CLM-${i}`,
        claim: `This is detailed fact number ${i} establishing specific technical properties of the system with sufficient word length.`,
        supportingSources: [`S${i}`],
        status: 'supported',
        confidence: 0.8 + (i % 20) * 0.01,
      });
    }

    const config = {
      ...DEFAULT_CONFIG.context,
      totalInputBudget: 2000,
      evidenceTokenBudget: 500,
    };

    const allocation = allocateEvidenceBudget(claims, 'What are the technical specs?', config);

    expect(allocation.tokenStats.estimatedEvidenceTokens).toBeLessThanOrEqual(500);
    expect(allocation.tokenStats.finalPromptTokens).toBeLessThanOrEqual(config.totalInputBudget);
    expect(allocation.droppedCount).toBeGreaterThan(0);
    // Dropped complete units, retained units are intact
    expect(allocation.claims.length).toBeGreaterThan(0);
    expect(allocation.claims.length).toBeLessThan(claims.length);
  });
});
