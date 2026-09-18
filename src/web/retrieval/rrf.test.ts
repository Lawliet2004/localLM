import { describe, it, expect } from 'vitest';
import { reciprocalRankFusion, type RankedItem } from './rrf';

describe('Reciprocal Rank Fusion (RRF)', () => {
  it('combines lexical and semantic rankings fairly', () => {
    interface Item { id: string; name: string }

    const itemA: Item = { id: 'A', name: 'Document A' };
    const itemB: Item = { id: 'B', name: 'Document B' };
    const itemC: Item = { id: 'C', name: 'Document C' };

    // Ranking 1: A (rank 1), B (rank 2)
    const rank1: Array<RankedItem<Item>> = [
      { item: itemA, rank: 1 },
      { item: itemB, rank: 2 },
    ];

    // Ranking 2: B (rank 1), C (rank 2), A (rank 3)
    const rank2: Array<RankedItem<Item>> = [
      { item: itemB, rank: 1 },
      { item: itemC, rank: 2 },
      { item: itemA, rank: 3 },
    ];

    const fused = reciprocalRankFusion([rank1, rank2], 60);

    // B has rank 2 + rank 1 = 1/62 + 1/61 = 0.0161 + 0.0163 = 0.0325
    // A has rank 1 + rank 3 = 1/61 + 1/63 = 0.0163 + 0.0158 = 0.0322
    // B should come out on top!
    expect(fused[0].item.id).toBe('B');
    expect(fused[1].item.id).toBe('A');
    expect(fused[2].item.id).toBe('C');
  });
});
