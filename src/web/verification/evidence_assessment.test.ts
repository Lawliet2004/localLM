import { expect, it } from 'vitest';
import { detectConservativeConflicts } from './evidence_assessment';
import type { EvidenceClaim } from '../types';

function assess(first: string, second: string) {
  const claims: EvidenceClaim[] = [first, second].map((claim, index) => ({ id: `E${index}`, claim,
    supportingSources: [`S${index}`], confidence: 0.8, status: 'supported' }));
  return detectConservativeConflicts(claims);
}

it('detects an exact quantity change or negation in the same assertion', () => {
  expect(assess('The adapter supplies 60 watts.', 'The adapter supplies 90 watts.').every(claim => claim.status === 'conflicting')).toBe(true);
  expect(assess('The adapter is compatible.', 'The adapter is not compatible.').every(claim => claim.status === 'conflicting')).toBe(true);
  expect(assess('The adapter is compatible.', "The adapter isn't compatible.").every(claim => claim.status === 'conflicting')).toBe(true);
});

it('treats metric values before a predicate as quantities while keeping numeric product identities', () => {
  expect(assess('The ticket price $60 is current.', 'The ticket price $90 is current.').every(claim => claim.status === 'conflicting')).toBe(true);
  expect(assess('Llama 3 has 8B parameters.', 'Llama 4 has 9B parameters.').every(claim => claim.status === 'supported')).toBe(true);
  expect(assess('React 18 was released in 2022.', 'React 19 was released in 2024.').every(claim => claim.status === 'supported')).toBe(true);
});

it('does not merge distinct benchmark versions, models, or evaluation conditions into a conflict', () => {
  expect(assess('Benchmark Code-Bench 1: Alpha 60%.', 'Benchmark Code-Bench 2: Alpha 90%.').every(claim => claim.status === 'supported')).toBe(true);
  expect(assess('Benchmark Code-Bench: Alpha 3 60%.', 'Benchmark Code-Bench: Alpha 4 90%.').every(claim => claim.status === 'supported')).toBe(true);
  expect(assess('Benchmark Code-Bench: Alpha 60%. Without tools.', 'Benchmark Code-Bench: Alpha 90%. With tools.').every(claim => claim.status === 'supported')).toBe(true);
});

it('preserves contradictory benchmark cells and leaves input records unchanged', () => {
  const input: EvidenceClaim[] = [{ id: 'A', claim: 'Benchmark Code-Bench: Alpha 60%; Beta 70%.', supportingSources: ['S1'], confidence: 0.8, status: 'supported' },
    { id: 'B', claim: 'Benchmark Code-Bench: Alpha 90%.', supportingSources: ['S2'], confidence: 0.8, status: 'supported' }];
  const assessed = detectConservativeConflicts(input);
  expect(assessed.every(claim => claim.status === 'conflicting')).toBe(true);
  expect(assessed[0].conflictingSources).toEqual(['S2']);
  expect(assessed[0].variants).toEqual(input.map(claim => claim.claim));
  expect(input.every(claim => claim.status === 'supported')).toBe(true);
});

it('does not report disagreement for equivalent decimal or thousands formatting', () => {
  expect(assess('The adapter supplies 60 watts.', 'The adapter supplies 60.0 watts.').every(claim => claim.status === 'supported')).toBe(true);
  expect(assess('The grant has 1,000 dollars.', 'The grant has 1000 dollars.').every(claim => claim.status === 'supported')).toBe(true);
  expect(assess('Benchmark Code-Bench: Alpha 60%.', 'Benchmark Code-Bench: Alpha 60.0%.').every(claim => claim.status === 'supported')).toBe(true);
});
