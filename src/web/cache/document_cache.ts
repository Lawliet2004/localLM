/**
 * Document Cache: Reuses extracted pages only within the requested freshness budget.
 */

import type { RetrievedDocument } from '../types';
import { InMemoryStorageAdapter, type StorageAdapter } from './sqlite';
import { normalizeUrl } from '../ranking/url_normalizer';

export class DocumentCache {
  constructor(private storage: StorageAdapter = new InMemoryStorageAdapter()) {}

  async get(url: string, maxAgeSeconds = Infinity): Promise<RetrievedDocument | null> {
    const document = await this.storage.get<RetrievedDocument>('document_cache', normalizeUrl(url));
    if (document && Number.isFinite(maxAgeSeconds)) {
      const age = Date.now() - Date.parse(document.retrievedAt);
      if (!Number.isFinite(age) || age < 0 || age >= maxAgeSeconds * 1000) return null;
    }
    return document;
  }

  async set(doc: RetrievedDocument, ttlSeconds: number = 86400 * 3, requestedUrl = doc.url): Promise<void> {
    const norm = normalizeUrl(requestedUrl);
    await this.storage.set('document_cache', norm, doc, ttlSeconds);
  }
}

