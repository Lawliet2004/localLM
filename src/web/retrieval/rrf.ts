/**
 * Reciprocal Rank Fusion (RRF): Combines multiple ranked candidate lists
 * (e.g. BM25 lexical rank and dense vector embedding rank) without needing
 * score calibration across different scales.
 */


export interface RankedItem<T> {
  item: T;
  rank: number; // 1-indexed
  score?: number;
}

export function reciprocalRankFusion<T extends { id: string }>(
  rankings: Array<Array<RankedItem<T>>>,
  k: number = 60
): Array<{ item: T; rrfScore: number }> {
  const scoreMap = new Map<string, { item: T; rrfScore: number }>();

  for (const ranking of rankings) {
    for (const entry of ranking) {
      const id = entry.item.id;
      const contribution = 1.0 / (k + entry.rank);

      const existing = scoreMap.get(id);
      if (existing) {
        existing.rrfScore += contribution;
      } else {
        scoreMap.set(id, {
          item: entry.item,
          rrfScore: contribution,
        });
      }
    }
  }

  const result = Array.from(scoreMap.values());
  result.sort((a, b) => b.rrfScore - a.rrfScore);
  return result;
}
