/**
 * Reranker Service: Coordinates passage reranking with fallback on weak hardware.
 */

import type { EvidenceChunk, Reranker } from '../types';
import { LocalCrossEncoderReranker } from './cross_encoder';

export class RerankerService implements Reranker {
  private crossEncoder: LocalCrossEncoderReranker;
  private enabled: boolean;

  constructor(enabled: boolean = true) {
    this.enabled = enabled;
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

    return this.crossEncoder.rerank(query, chunks, limit);
  }
}
