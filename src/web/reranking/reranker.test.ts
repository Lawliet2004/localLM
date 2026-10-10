import { expect, it } from 'vitest';
import { applyRecencyBlend, recencyScore, RerankerService } from './reranker';
import type { EvidenceChunk } from '../types';

function chunk(id: string, score: number, publishedAt?: string): EvidenceChunk {
  return {
    id,
    documentId: id,
    url: `https://example.com/${id}`,
    title: id,
    text: 'passage',
    tokenCount: 8,
    rerankScore: score,
    publishedAt,
  };
}

it('scores recent dates near 1 and missing dates as 0', () => {
  expect(recencyScore(new Date().toISOString())).toBeGreaterThan(0.9);
  expect(recencyScore(undefined)).toBe(0);
  expect(recencyScore('not-a-date')).toBe(0);
});

it('boosts dated chunks without punishing undated ones', () => {
  const ranked = applyRecencyBlend([
    chunk('old', 0.8, '2020-01-01'),
    chunk('new', 0.8, new Date().toISOString()),
    chunk('undated', 0.85),
  ], 0.15);
  expect(ranked[0].id).toBe('new');
  expect(ranked.find((c) => c.id === 'undated')?.rerankScore).toBe(0.85);
});

it('reranks the full candidate pool by query relevance before selecting passages', async () => {
  const unrelated = { ...chunk('irrelevant', 0), text: 'The weather in London is rainy.' };
  const relevant = { ...chunk('relevant', 0), text: 'React stable release adds actions and server components.' };
  const ranked = await new RerankerService(true).rerank('React stable release', [unrelated, relevant], 1);
  expect(ranked.map(item => item.id)).toEqual(['relevant']);
  expect(ranked[0].rerankScore).toBeGreaterThan(0);
  expect((await new RerankerService(false).rerank('React stable release', [unrelated, relevant], 1))[0].id).toBe('irrelevant');
});
