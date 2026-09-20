/**
 * Reranker Service: Coordinates passage reranking with fallback on weak hardware.
 * Optionally blends published-date recency into rerank scores (exponential
 * decay, 90-day scale) — skipped when the caller already filtered by a
 * freshness window, mirroring the search-stage behavior.
 */

import type { EvidenceChunk, Reranker } from '../types';
import { LocalCrossEncoderReranker } from './cross_encoder';
/** Exponential recency decay over a 90-day scale; 0 for missing or future dates. */
export function recencyScore(publishedAt?: string): number {
  if (!publishedAt) return 0;
  const ms = Date.parse(publishedAt);
  if (Number.isNaN(ms)) return 0;
  const ageDays = (Date.now() - ms) / 86_400_000;
  if (ageDays < 0) return 0;
  return Math.exp(-ageDays / 90);
}

/**
 * Adds `weight * recencyScore` to each chunk's rerank score and re-sorts.
 * Undated chunks keep their raw score — they are neither punished nor boosted.
 */
export function applyRecencyBlend(
  chunks: EvidenceChunk[],
  weight: number,
): EvidenceChunk[] {
  if (!weight || weight <= 0 || chunks.length <= 1) return chunks;
  return chunks
    .map((c) => {
      const recency = recencyScore(c.publishedAt);
      if (recency === 0) return c;
      return { ...c, rerankScore: Math.min(1.15, (c.rerankScore ?? 0) + weight * recency) };
    })
    .sort((a, b) => (b.rerankScore ?? 0) - (a.rerankScore ?? 0));
}

export class RerankerService implements Reranker {
  private crossEncoder: LocalCrossEncoderReranker;
  private enabled: boolean;
  private recencyWeight: number;

  constructor(enabled: boolean = true, recencyWeight: number = 0) {
    this.enabled = enabled;
    this.recencyWeight = recencyWeight;
    this.crossEncoder = new LocalCrossEncoderReranker();
  }

  async rerank(
    query: string,
    chunks: EvidenceChunk[],
    limit: number = 8
  ): Promise<EvidenceChunk[]> {
    if (!this.enabled) {
      return chunks.slice(0, limit);
    }

    // Score the full pool (the heuristic scorer early-returns unsorted when
    // limit >= chunk count), blend recency, then slice to the final limit.
    const ranked = await this.crossEncoder.rerank(query, chunks, Number.MAX_SAFE_INTEGER);
    const blended = applyRecencyBlend(ranked, this.recencyWeight);
    return blended.slice(0, limit);
  }
}

/**
 * Wraps a model-backed reranker (e.g. a llama.cpp /reranking cross-encoder)
 * with the lexical fallback: an unavailable or erroring reranker degrades to
 * heuristic ordering instead of failing the whole research turn. A single
 * throttled notice marks the degradation so ranking-quality drops stay
 * answerable from logs.
 */
export class ResilientReranker implements Reranker {
  private warned = false;
  private heuristic: LocalCrossEncoderReranker;
  constructor(
    private primary: Reranker,
    private recencyWeight: number = 0,
    private onDegrade?: (error: unknown) => void,
  ) {
    this.heuristic = new LocalCrossEncoderReranker();
  }

  async rerank(query: string, chunks: EvidenceChunk[], limit: number): Promise<EvidenceChunk[]> {
    if (chunks.length === 0) return [];
    let ranked: EvidenceChunk[];
    try {
      ranked = await this.primary.rerank(query, chunks, Number.MAX_SAFE_INTEGER);
      if (!Array.isArray(ranked) || ranked.length === 0) throw new Error('reranker returned no results');
    } catch (error) {
      if (!this.warned) {
        this.warned = true;
        this.onDegrade?.(error);
      }
      ranked = await this.heuristic.rerank(query, chunks, Number.MAX_SAFE_INTEGER);
    }
    return applyRecencyBlend(ranked, this.recencyWeight).slice(0, limit);
  }
}
