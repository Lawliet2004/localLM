/**
 * Google Custom Search Provider: Interfaces with the Google Custom Search JSON API.
 * Free tier: 100 queries/day. Requires API key and Custom Search Engine ID.
 */

import type { SearchProvider, SearchRequest, SearchResult } from '../types';

export class GoogleSearchProvider implements SearchProvider {
  constructor(
    private apiKey: string,
    private cxId: string,
    private timeoutMs: number = 8000
  ) {}

  async search(request: SearchRequest): Promise<SearchResult[]> {
    const url = new URL('https://www.googleapis.com/customsearch/v1');
    url.searchParams.set('key', this.apiKey);
    url.searchParams.set('cx', this.cxId);
    url.searchParams.set('q', request.query);
    url.searchParams.set('num', String(Math.min(request.maxResults ?? 10, 10)));
    if (request.language) url.searchParams.set('lr', `lang_${request.language}`);

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const res = await fetch(url.toString(), {
        headers: { Accept: 'application/json' },
        signal: controller.signal,
        redirect: 'error',
      });

      if (!res.ok) {
        const body = await res.text();
        throw new Error(`Google Search returned HTTP ${res.status}: ${body.slice(0, 200)}`);
      }

      const data = await res.json();
      clearTimeout(timer);
      if (!data || !Array.isArray(data.items)) {
        return [];
      }

      const limit = request.maxResults ?? 10;
      const results: SearchResult[] = [];
      for (let i = 0; i < Math.min(data.items.length, limit); i++) {
        const item = data.items[i];
        results.push({
          id: `google-${i + 1}`,
          queryId: request.query,
          title: item.title || '',
          url: item.link || '',
          snippet: item.snippet || '',
          domain: '',
          publishedAt: undefined,
          engine: 'google',
          rank: i + 1,
          score: undefined,
          metadata: {},
        });
      }
      return results;
    } catch (err: any) {
      clearTimeout(timer);
      if (err.name === 'AbortError') {
        throw new Error(`Google Search request timed out after ${this.timeoutMs}ms`);
      }
      throw err;
    }
  }
}
