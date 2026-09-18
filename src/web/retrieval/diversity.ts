/**
 * Diversity Control: Implements per-document chunk limits and Maximal Marginal
 * Relevance (MMR) to prevent a single document from dominating retrieved passages.
 */

import type { EvidenceChunk } from '../types';
import { tokenJaccardSimilarity } from '../ranking/deduplicator';

export interface DiversityOptions {
  maxChunksPerDocument?: number;
  lambda?: number; // MMR tradeoff: 1 = pure relevance, 0 = pure diversity (default 0.7)
}

/**
 * Filters chunks to ensure no single document contributes more than maxChunksPerDocument.
 */
export function applyDocumentDiversity(
  chunks: EvidenceChunk[],
  maxChunksPerDoc: number = 2
): EvidenceChunk[] {
  const docCounts = new Map<string, number>();
  const filtered: EvidenceChunk[] = [];

  for (const chunk of chunks) {
    const count = docCounts.get(chunk.documentId) ?? 0;
    if (count < maxChunksPerDoc) {
      docCounts.set(chunk.documentId, count + 1);
      filtered.push(chunk);
    }
  }

  return filtered;
}

/**
 * Maximal Marginal Relevance (MMR) re-ordering of evidence chunks.
 */
export function maximalMarginalRelevance(
  query: string,
  candidates: EvidenceChunk[],
  limit: number,
  lambda: number = 0.7
): EvidenceChunk[] {
  if (candidates.length <= limit) {
    return candidates;
  }

  const selected: EvidenceChunk[] = [];
  const remaining = [...candidates];

  while (selected.length < limit && remaining.length > 0) {
    let bestScore = -Infinity;
    let bestIndex = -1;

    for (let i = 0; i < remaining.length; i++) {
      const candidate = remaining[i];
      // Relevance to query: use prior score (semantic/lexical) or Jaccard
      const queryRel = candidate.semanticScore ?? tokenJaccardSimilarity(candidate.text, query);

      // Max redundancy with already selected chunks
      let maxRedundancy = 0;
      for (const sel of selected) {
        const sim = tokenJaccardSimilarity(candidate.text, sel.text);
        if (sim > maxRedundancy) {
          maxRedundancy = sim;
        }
      }

      // MMR score formula
      const mmrScore = lambda * queryRel - (1 - lambda) * maxRedundancy;
      if (mmrScore > bestScore) {
        bestScore = mmrScore;
        bestIndex = i;
      }
    }

    if (bestIndex >= 0) {
      selected.push(remaining[bestIndex]);
      remaining.splice(bestIndex, 1);
    } else {
      break;
    }
  }

  return selected;
}
