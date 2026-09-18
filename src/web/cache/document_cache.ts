/**
 * Document Cache: Stores extracted webpage text and metadata with content hashing.
 */

import type { RetrievedDocument } from '../types';
import { InMemoryStorageAdapter, type StorageAdapter } from './sqlite';
import { normalizeUrl } from '../ranking/url_normalizer';

export class DocumentCache {
  constructor(private storage: StorageAdapter = new InMemoryStorageAdapter()) {}

  async get(url: string): Promise<RetrievedDocument | null> {
    const norm = normalizeUrl(url);
    return this.storage.get<RetrievedDocument>('document_cache', norm);
  }

  async set(doc: RetrievedDocument, ttlSeconds: number = 86400 * 3): Promise<void> {
    const norm = normalizeUrl(doc.url);
    await this.storage.set('document_cache', norm, doc, ttlSeconds);
  }
}

