/**
 * Embedding Cache: Caches embedding vectors keyed by model and content hash.
 */

import { InMemoryStorageAdapter, type StorageAdapter } from './sqlite';
import { computeContentHash } from '../extraction/main_content';

export class EmbeddingCache {
  constructor(private storage: StorageAdapter = new InMemoryStorageAdapter()) {}

  private makeKey(modelId: string, text: string): string {
    const hash = computeContentHash(text);
    return `${modelId}::${hash}`;
  }

  async get(modelId: string, text: string): Promise<number[] | null> {
    const key = this.makeKey(modelId, text);
    return this.storage.get<number[]>('embedding_cache', key);
  }

  async set(modelId: string, text: string, vector: number[]): Promise<void> {
    const key = this.makeKey(modelId, text);
    // Embeddings don't expire quickly (store for 30 days)
    await this.storage.set('embedding_cache', key, vector, 86400 * 30);
  }
}

