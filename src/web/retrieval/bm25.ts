/**
 * In-Memory Okapi BM25: Fast, lightweight lexical search for retrieved webpage chunks
 * without requiring Elasticsearch, Lucene, or cloud services.
 */

import type { EvidenceChunk } from '../types';

export interface BM25Options {
  k1?: number; // term frequency saturation (default 1.5)
  b?: number; // document length normalization (default 0.75)
}

export class InMemoryBM25 {
  private k1: number;
  private b: number;
  private docCount: number = 0;
  private avgDocLength: number = 0;
  private docLengths: number[] = [];
  private termFrequencies: Array<Map<string, number>> = [];
  private docFrequencies: Map<string, number> = new Map();
  private chunks: EvidenceChunk[] = [];

  constructor(options: BM25Options = {}) {
    this.k1 = options.k1 ?? 1.5;
    this.b = options.b ?? 0.75;
  }

  private tokenize(text: string): string[] {
    return text
      .toLowerCase()
      .replace(/[^\w\s]/g, ' ')
      .split(/\s+/)
      .filter((t) => t.length > 1);
  }

  buildIndex(chunks: EvidenceChunk[]) {
    this.chunks = chunks;
    this.docCount = chunks.length;
    this.docLengths = [];
    this.termFrequencies = [];
    this.docFrequencies.clear();

    if (this.docCount === 0) {
      this.avgDocLength = 0;
      return;
    }

    let totalLength = 0;

    for (const chunk of chunks) {
      const tokens = this.tokenize(chunk.text);
      const len = tokens.length;
      this.docLengths.push(len);
      totalLength += len;

      const tf = new Map<string, number>();
      for (const token of tokens) {
        tf.set(token, (tf.get(token) ?? 0) + 1);
      }
      this.termFrequencies.push(tf);

      for (const term of tf.keys()) {
        this.docFrequencies.set(term, (this.docFrequencies.get(term) ?? 0) + 1);
      }
    }

    this.avgDocLength = totalLength / this.docCount;
  }

  search(query: string, limit?: number): Array<{ chunk: EvidenceChunk; score: number }> {
    if (this.docCount === 0) return [];

    const queryTokens = this.tokenize(query);
    if (queryTokens.length === 0) return [];

    const scores: Array<{ chunk: EvidenceChunk; score: number }> = [];

    for (let i = 0; i < this.docCount; i++) {
      const tfMap = this.termFrequencies[i];
      const docLen = this.docLengths[i];
      let score = 0;

      for (const term of queryTokens) {
        const tf = tfMap.get(term) ?? 0;
        if (tf === 0) continue;

        const df = this.docFrequencies.get(term) ?? 0;
        // Standard Okapi BM25 IDF: ln((N - df + 0.5) / (df + 0.5) + 1)
        const idf = Math.log(1 + (this.docCount - df + 0.5) / (df + 0.5));

        // Term frequency component with length normalization
        const num = tf * (this.k1 + 1);
        const denom = tf + this.k1 * (1 - this.b + this.b * (docLen / (this.avgDocLength || 1)));

        score += idf * (num / denom);
      }

      if (score > 0) {
        scores.push({
          chunk: { ...this.chunks[i], lexicalScore: Number(score.toFixed(4)) },
          score,
        });
      }
    }

    scores.sort((a, b) => b.score - a.score);
    return limit ? scores.slice(0, limit) : scores;
  }
}
