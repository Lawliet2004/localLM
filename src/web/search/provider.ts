/**
 * Search Provider Interface: Decouples search backends from engine consumers.
 */

import type { SearchRequest, SearchResult, SearchProvider } from '../types';

export type { SearchRequest, SearchResult, SearchProvider };

export class MockSearchProvider implements SearchProvider {
  constructor(private resultsMap: Record<string, SearchResult[]> = {}) {}

  async search(request: SearchRequest): Promise<SearchResult[]> {
    const qLower = request.query.toLowerCase();
    for (const [key, results] of Object.entries(this.resultsMap)) {
      if (qLower.includes(key.toLowerCase()) || key.toLowerCase().includes(qLower)) {
        return results;
      }
    }
    return [];
  }
}
