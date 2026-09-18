/**
 * Hybrid Retriever: Combines BM25 lexical search with dense vector embeddings
 * using Reciprocal Rank Fusion (RRF) and diversity filtering.
 */

import type { EvidenceChunk, EmbeddingProvider } from '../types';
import { InMemoryBM25 } from './bm25';
import { cosineSimilarity } from './embeddings';
import { reciprocalRankFusion, type RankedItem } from './rrf';
import { applyDocumentDiversity } from './diversity';

export interface HybridRetrieverOptions {
  bm25Enabled?: boolean;
  embeddingsEnabled?: boolean;
  rrfK?: number;
  candidateLimit?: number;
  maxChunksPerDoc?: number;
}

export class HybridRetriever {
  private bm25: InMemoryBM25;

  constructor(
    private embeddingProvider: EmbeddingProvider,
    private options: HybridRetrieverOptions = {}
  ) {
    this.bm25 = new InMemoryBM25();
  }

  async retrieve(
    query: string,
    chunks: EvidenceChunk[],
    limit: number = 25
  ): Promise<EvidenceChunk[]> {
    if (chunks.length === 0) return [];

    const useBm25 = this.options.bm25Enabled ?? true;
    const useEmbeddings = this.options.embeddingsEnabled ?? true;
    const rrfK = this.options.rrfK ?? 60;
    const maxPerDoc = this.options.maxChunksPerDoc ?? 2;

    const rankings: Array<Array<RankedItem<EvidenceChunk>>> = [];

    // 1. BM25 Lexical Retrieval
    if (useBm25) {
      this.bm25.buildIndex(chunks);
      const bm25Hits = this.bm25.search(query, chunks.length);
      const bm25Ranked: Array<RankedItem<EvidenceChunk>> = bm25Hits.map((hit, idx) => ({
        item: hit.chunk,
        rank: idx + 1,
        score: hit.score,
      }));
      rankings.push(bm25Ranked);
    }

    // 2. Dense Vector Retrieval
    if (useEmbeddings) {
      const queryVec = await this.embeddingProvider.embedQuery(query);
      const docTexts = chunks.map((c) => c.text);
      const docVecs = await this.embeddingProvider.embedDocuments(docTexts);

      const denseScores = chunks.map((chunk, i) => {
        const sim = cosineSimilarity(queryVec, docVecs[i]);
        return {
          chunk: { ...chunk, semanticScore: Number(sim.toFixed(4)) },
          score: sim,
        };
      });

      denseScores.sort((a, b) => b.score - a.score);

      const denseRanked: Array<RankedItem<EvidenceChunk>> = denseScores.map((entry, idx) => ({
        item: entry.chunk,
        rank: idx + 1,
        score: entry.score,
      }));
      rankings.push(denseRanked);
    }

    // 3. Reciprocal Rank Fusion
    let fusedChunks: EvidenceChunk[];
    if (rankings.length > 1) {
      const fused = reciprocalRankFusion(rankings, rrfK);
      fusedChunks = fused.map((f) => ({
        ...f.item,
        rerankScore: Number(f.rrfScore.toFixed(4)),
      }));
    } else if (rankings.length === 1) {
      fusedChunks = rankings[0].map((r) => r.item);
    } else {
      fusedChunks = [...chunks];
    }

    // 4. Apply document diversity cap
    const diverse = applyDocumentDiversity(fusedChunks, maxPerDoc);

    return diverse.slice(0, limit);
  }
}
