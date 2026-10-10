/**
 * Google Custom Search Provider: Interfaces with the Google Custom Search JSON API.
 * Free tier: 100 queries/day. Requires API key and Custom Search Engine ID.
 */

import type { SearchProvider, SearchRequest, SearchResult } from '../types';

export class GoogleSearchProvider implements SearchProvider {
  constructor(
    private apiKey: string,
    private cxId: string,
    private timeoutMs: number = 8000,
    private claimQuota?: () => Promise<boolean>,
  ) {}

  async search(request: SearchRequest): Promise<SearchResult[]> {
    const url = new URL('https://www.googleapis.com/customsearch/v1');
    url.searchParams.set('key', this.apiKey);
    url.searchParams.set('cx', this.cxId);
    url.searchParams.set('q', request.query);
    const num = Math.max(1, Math.min(Math.floor(request.maxResults ?? 10), 10));
    const page = Math.max(1, Math.floor(request.page ?? 1));
    const start = (page - 1) * num + 1;
    if (start > 100) return [];
    url.searchParams.set('num', String(Math.min(num, 101 - start)));
    url.searchParams.set('start', String(start));
    const freshness = { day: 'd1', week: 'w1', month: 'm1', year: 'y1' };
    if (request.freshness && request.freshness in freshness) url.searchParams.set('dateRestrict', freshness[request.freshness as keyof typeof freshness]);
    if (request.language) url.searchParams.set('lr', `lang_${request.language}`);

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      if (this.claimQuota && !(await this.claimQuota())) throw new Error('Google daily quota exhausted');
      const res = await fetch(url.toString(), {
        headers: { Accept: 'application/json' },
        signal: controller.signal,
        redirect: 'error',
      });

      if (!res.ok) {
        throw new Error(`Google Search returned HTTP ${res.status}`);
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
      const message = String(err?.message || err);
      throw new Error(this.apiKey ? message.split(this.apiKey).join('[redacted]').split(encodeURIComponent(this.apiKey)).join('[redacted]') : message);
    }
  }
}
