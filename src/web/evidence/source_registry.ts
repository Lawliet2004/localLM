/**
 * Source Registry: Manages deterministic source identifiers (S1, S2, ...)
 * and chunk mapping (S1:C1, S1:C2) so LLMs cannot hallucinate arbitrary URLs.
 */

import type { RetrievedDocument, SourceMeta, SourceRegistry } from '../types';
import { extractDomain } from '../ranking/url_normalizer';

export class SourceManager {
  private sources = new Map<string, SourceMeta>(); // e.g. "S1" -> SourceMeta
  private urlToId = new Map<string, string>(); // url -> "S1"
  private counter = 1;

  registerDocument(doc: RetrievedDocument): string {
    const existing = this.urlToId.get(doc.url);
    if (existing) return existing;

    const sourceId = `S${this.counter++}`;
    const meta: SourceMeta = {
      id: sourceId,
      title: doc.title || 'Untitled Document',
      url: doc.url,
      domain: doc.domain || extractDomain(doc.url),
      publishedAt: doc.publishedAt,
      author: doc.author,
      sourceType: doc.sourceType,
      authorityScore: doc.authorityScore,
    };

    this.sources.set(sourceId, meta);
    this.urlToId.set(doc.url, sourceId);
    return sourceId;
  }

  getSource(id: string): SourceMeta | undefined {
    return this.sources.get(id);
  }

  getSourceIdForUrl(url: string): string | undefined {
    return this.urlToId.get(url);
  }

  getAllSources(): SourceRegistry {
    const registry: SourceRegistry = {};
    for (const [k, v] of this.sources.entries()) {
      registry[k] = v;
    }
    return registry;
  }

  formatSourcesForContext(): string {
    const lines: string[] = [];
    for (const [id, meta] of this.sources.entries()) {
      const dateStr = meta.publishedAt ? ` (Published: ${meta.publishedAt})` : '';
      lines.push(`[${id}] "${meta.title}"${dateStr} - ${meta.url}`);
    }
    return lines.join('\n');
  }
}
