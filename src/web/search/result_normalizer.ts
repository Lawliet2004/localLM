/**
 * Result Normalizer: Standardizes raw SERP result items into uniform internal SearchResult objects.
 */

import type { SearchResult } from '../types';
import { extractDomain } from '../ranking/url_normalizer';

export function normalizeRawSearchResult(
  raw: any,
  queryId: string,
  rank: number,
  page: number = 1
): SearchResult {
  const url = String(raw.url || raw.link || '').trim();
  const title = String(raw.title || 'Untitled').trim();
  const snippet = String(raw.content || raw.snippet || raw.body || '').trim();
  const domain = extractDomain(url);
  const publishedAt = raw.publishedDate || raw.publishedAt || raw.date;

  return {
    id: page > 1 ? `${queryId}-P${page}-R${rank}` : `${queryId}-R${rank}`,
    queryId,
    title,
    url,
    snippet,
    domain,
    publishedAt: publishedAt ? String(publishedAt) : undefined,
    engine: raw.engine ? String(raw.engine) : undefined,
    rank: (page - 1) * 10 + rank,
    score: typeof raw.score === 'number' ? raw.score : undefined,
    metadata: { ...(raw.metadata || {}), ...(page > 1 ? { searxngPage: page } : {}) },
  };
}
