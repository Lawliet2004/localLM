/**
 * Result Fusion: Aggregates candidate search results from multiple queries into a single
 * candidate pool and runs multi-signal deduplication.
 */

import type { SearchResult } from '../types';
import { deduplicateSearchResults } from '../ranking/deduplicator';

export function fuseSearchResults(queryResults: Array<{ query: string; results: SearchResult[] }>): SearchResult[] {
  const allResults: SearchResult[] = [];

  for (const group of queryResults) {
    for (const r of group.results) {
      allResults.push({
        ...r,
        queryId: r.queryId || group.query,
      });
    }
  }

  return deduplicateSearchResults(allResults);
}
