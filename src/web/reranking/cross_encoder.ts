/**
 * Lexical fallback scorer. This is not a trained cross-encoder.
 * Use LocalModelReranker for model inference on query/passage pairs.
 */

import type { EvidenceChunk, Reranker } from '../types';
import { tokenJaccardSimilarity } from '../ranking/deduplicator';

export class LocalCrossEncoderReranker implements Reranker {
  private tokenize(text: string): string[] {
    return text
      .toLowerCase()
      .replace(/[^\w\s]/g, ' ')
      .split(/\s+/)
      .filter((t) => t.length > 1);
  }

  /**
   * Pairwise passage relevance scoring:
   * Examines query token coverage, ordered token sequences, proximity, and exact numbers/entities.
   */
  scorePassage(query: string, passageText: string): number {
    const qTokens = this.tokenize(query);
    const pLower = passageText.toLowerCase();

    if (qTokens.length === 0) return 0.5;

    // 1. Query token coverage
    let matched = 0;
    let exactMatches = 0;
    for (const token of qTokens) {
      if (pLower.includes(token)) {
        matched++;
        // Check word boundary exact match
        const regex = new RegExp(`\\b${token}\\b`, 'i');
        if (regex.test(pLower)) {
          exactMatches++;
        }
      }
    }
    const coverage = matched / qTokens.length;
    const exactRatio = exactMatches / qTokens.length;

    // 2. Bigram / phrase continuity
    let phraseMatches = 0;
    for (let i = 0; i < qTokens.length - 1; i++) {
      const phrase = `${qTokens[i]} ${qTokens[i + 1]}`;
      if (pLower.includes(phrase)) {
        phraseMatches++;
      }
    }
    const phraseBonus = qTokens.length > 1 ? (phraseMatches / (qTokens.length - 1)) * 0.25 : 0;

    // 3. Numbers, dates, version strings match
    const numbersAndVersions = query.match(/\b(\d+(\.\d+)*|\d{4})\b/g) || [];
    let numMatches = 0;
    for (const num of numbersAndVersions) {
      if (pLower.includes(num.toLowerCase())) {
        numMatches++;
      }
    }
    const numBonus = numbersAndVersions.length > 0 ? (numMatches / numbersAndVersions.length) * 0.2 : 0;

    // 4. Token Jaccard overlap
    const jaccard = tokenJaccardSimilarity(query, passageText);

    // Combined cross-encoder score in [0.0 - 1.0]
    const rawScore = 0.40 * coverage + 0.25 * exactRatio + 0.15 * phraseBonus + 0.10 * numBonus + 0.10 * jaccard;
    return Number(Math.min(1.0, Math.max(0.0, rawScore)).toFixed(4));
  }

  async rerank(
    query: string,
    chunks: EvidenceChunk[],
    limit: number = 8
  ): Promise<EvidenceChunk[]> {
    if (chunks.length <= limit) {
      return chunks;
    }

    const scored = chunks.map((chunk) => {
      const score = this.scorePassage(query, chunk.text);
      return {
        ...chunk,
        rerankScore: score,
      };
    });

    scored.sort((a, b) => (b.rerankScore ?? 0) - (a.rerankScore ?? 0));
    return scored.slice(0, limit);
  }
}
