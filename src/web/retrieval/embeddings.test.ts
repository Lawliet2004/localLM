import { describe, it, expect } from 'vitest';
import { LocalHashingEmbeddingProvider, cosineSimilarity } from './embeddings';

describe('Local Embeddings & Cosine Similarity', () => {
  it('computes cosine similarity accurately', () => {
    const v1 = [1, 0, 0];
    const v2 = [1, 0, 0];
    const v3 = [0, 1, 0];

    expect(cosineSimilarity(v1, v2)).toBeCloseTo(1.0);
    expect(cosineSimilarity(v1, v3)).toBeCloseTo(0.0);
  });

  it('produces higher similarity for semantically related texts', async () => {
    const provider = new LocalHashingEmbeddingProvider();

    const q = await provider.embedQuery('small coding language models');
    const docRelated = await provider.embedQuery('Qwen and DeepSeek coding models under 10B parameters');
    const docUnrelated = await provider.embedQuery('baking sourdough bread with yeast');

    const simRelated = cosineSimilarity(q, docRelated);
    const simUnrelated = cosineSimilarity(q, docUnrelated);

    expect(simRelated).toBeGreaterThan(simUnrelated);
  });
});
