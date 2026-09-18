import { expect, it } from 'vitest';
import { buildModelBudgetedContext } from './context_builder';
import { DEFAULT_CONFIG } from '../config/defaults';
import type { EvidenceClaim } from '../types';

it('drops whole facts when runtime token counts exceed the planning estimate', async () => {
  const claims: EvidenceClaim[] = Array.from({ length: 12 }, (_, i) => ({
    id: `c${i}`, claim: `Fact ${i}: ` + 'The release adds useful improvements. '.repeat(6),
    supportingSources: [], status: 'supported', confidence: 1 - i / 100,
  }));
  const count = async (text: string) => new TextEncoder().encode(text).length;
  const result = await buildModelBudgetedContext('Latest release?', {}, claims, DEFAULT_CONFIG.context, '2026-09-17', count);
  expect(result.allocation.claims.length).toBeLessThan(claims.length);
  expect(result.allocation.claims.length).toBeGreaterThan(0);
  expect(await count(result.context.userPrompt)).toBeLessThanOrEqual(2500);
  expect(result.allocation.tokenStats.finalPromptTokens + DEFAULT_CONFIG.context.safetyMargin).toBeLessThanOrEqual(6000);
  expect(result.allocation.claims.every(c => claims.some(original => original.claim === c.claim))).toBe(true);
});
