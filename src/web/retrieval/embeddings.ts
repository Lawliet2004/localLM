/**
 * Local Embeddings: Zero-cost dense representation and cosine similarity.
 * Runs completely locally on CPU with zero external API requirements.
 */

import type { EmbeddingProvider } from '../types';

/**
 * Computes cosine similarity between two unit vectors.
 */
export function cosineSimilarity(a: number[], b: number[]): number {
  if (a.length !== b.length || a.length === 0) return 0;

  let dot = 0;
  let normA = 0;
  let normB = 0;

  for (let i = 0; i < a.length; i++) {
    dot += a[i] * b[i];
    normA += a[i] * a[i];
    normB += b[i] * b[i];
  }

  if (normA === 0 || normB === 0) return 0;
  return dot / (Math.sqrt(normA) * Math.sqrt(normB));
}

/**
 * Fast, CPU-friendly deterministic dense vectorizer using subword feature hashing
 * and n-gram projections. Produces 256-dimensional unit vectors.
 */
export class LocalHashingEmbeddingProvider implements EmbeddingProvider {
  private dimensions: number;

  constructor(dimensions: number = 1024) {
    this.dimensions = dimensions;
  }

  private hashToken(token: string, seed: number): number {
    let h = seed >>> 0;
    for (let i = 0; i < token.length; i++) {
      h = Math.imul(h ^ token.charCodeAt(i), 16777619) >>> 0;
    }
    return (h >>> 0) % this.dimensions;
  }

  private vectorize(text: string): number[] {
    const vec = new Array(this.dimensions).fill(0);
    const stopWords = new Set(['and', 'with', 'the', 'of', 'in', 'to', 'for', 'is', 'a', 'an', 'on', 'at', 'by']);

    const tokens = text
      .toLowerCase()
      .replace(/[^\w\s]/g, ' ')
      .split(/\s+/)
      .filter((t) => t.length > 0 && !stopWords.has(t));

    if (tokens.length === 0) {
      return vec;
    }

    // 1. Unigram word features
    for (let i = 0; i < tokens.length; i++) {
      const word = tokens[i];
      const idx = this.hashToken(word, 0x811c9dc5);
      vec[idx] += 3.0;

      // 2. Character 3-grams for subword similarity
      if (word.length >= 3) {
        for (let j = 0; j <= word.length - 3; j++) {
          const tri = word.substring(j, j + 3);
          const triIdx = this.hashToken(tri, 0x9e3779b9);
          vec[triIdx] += 1.0;
        }
      }

      // 3. Word bigrams
      if (i < tokens.length - 1) {
        const bigram = `${word}_${tokens[i + 1]}`;
        const biIdx = this.hashToken(bigram, 0xbf597fc7);
        vec[biIdx] += 2.0;
      }
    }

    // L2 normalization to unit length
    let norm = 0;
    for (let i = 0; i < this.dimensions; i++) {
      norm += vec[i] * vec[i];
    }
    const mag = Math.sqrt(norm);
    if (mag > 0) {
      for (let i = 0; i < this.dimensions; i++) {
        vec[i] = Number((vec[i] / mag).toFixed(6));
      }
    }

    return vec;
  }

  async embedQuery(text: string): Promise<number[]> {
    return this.vectorize(text);
  }

  async embedDocuments(texts: string[]): Promise<number[][]> {
    return texts.map((t) => this.vectorize(t));
  }
}

/**
 * HTTP Embedding provider for connecting to local Ollama or llama-server endpoints.
 */
export class LocalHttpEmbeddingProvider implements EmbeddingProvider {
  constructor(
    private endpointUrl: string = 'http://127.0.0.1:11434/api/embeddings',
    private modelName: string = 'nomic-embed-text'
  ) {}

  async embedQuery(text: string): Promise<number[]> {
    try {
      const res = await fetch(this.endpointUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ model: this.modelName, prompt: text }),
      });
      if (!res.ok) throw new Error(`Embedding failed with ${res.status}`);
      const data = await res.json();
      return data.embedding || [];
    } catch {
      // Fallback to local hashing if HTTP server is unavailable
      const fallback = new LocalHashingEmbeddingProvider();
      return fallback.embedQuery(text);
    }
  }

  async embedDocuments(texts: string[]): Promise<number[][]> {
    return Promise.all(texts.map((t) => this.embedQuery(t)));
  }
}
