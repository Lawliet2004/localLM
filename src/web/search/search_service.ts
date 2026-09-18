/**
 * Search Service: Executes multiple search queries concurrently with bounded concurrency,
 * failure isolation (Promise.allSettled), and result fusion.
 *
 * Query budgets, concurrency, retries, and pagination are explicit: one
 * failed engine/query never discards healthy results, and failures are
 * reported per query instead of failing the whole batch.
 */

import type { PlannedQuery, SearchProvider, SearchResult } from '../types';
import { fuseSearchResults } from './result_fusion';

export interface SearchServiceOptions {
  maxConcurrentQueries?: number;
  retries?: number;
  retryDelayMs?: number;
}

export interface QueryFailure {
  query: string;
  error: string;
}

export class SearchService {
  private maxConcurrent: number;
  private retries: number;
  private retryDelayMs: number;
  constructor(
    private provider: SearchProvider,
    options: SearchServiceOptions = {},
  ) {
    this.maxConcurrent = Math.max(1, Math.min(8, Math.floor(options.maxConcurrentQueries ?? 4) || 4));
    this.retries = Math.max(0, Math.min(3, Math.floor(options.retries ?? 1) || 0));
    this.retryDelayMs = Math.max(0, Math.min(10000, options.retryDelayMs ?? 1000));
  }

  private async searchOnce(query: PlannedQuery, maxResults: number): Promise<SearchResult[]> {
    let lastError = '';
    for (let attempt = 0; attempt <= this.retries; attempt++) {
      try {
        return await this.provider.search({
          query: query.query,
          freshness: query.freshness,
          maxResults,
          page: query.page,
        });
      } catch (err: any) {
        lastError = String(err?.message || err);
        if (/retry-after/i.test(lastError)) break;
        if (attempt < this.retries) await new Promise((r) => setTimeout(r, this.retryDelayMs));
      }
    }
    throw new Error(lastError || 'search failed');
  }

  async executeSearches(
    queries: PlannedQuery[],
    maxResultsPerQuery: number = 10
  ): Promise<{ results: SearchResult[]; rawCount: number; failureCount: number; failures: QueryFailure[] }> {
    if (queries.length === 0) {
      return { results: [], rawCount: 0, failureCount: 0, failures: [] };
    }

    const successfulGroups: Array<{ query: string; results: SearchResult[] }> = [];
    const failures: QueryFailure[] = [];
    let rawCount = 0;

    const queue = [...queries];
    const workers = Array.from({ length: Math.min(this.maxConcurrent, queue.length) }, async () => {
      while (queue.length > 0) {
        const q = queue.shift()!;
        try {
          const hits = await this.searchOnce(q, maxResultsPerQuery);
          successfulGroups.push({ query: q.query, results: hits });
          rawCount += hits.length;
        } catch (err: any) {
          failures.push({ query: q.query, error: String(err?.message || err) });
        }
      }
    });
    await Promise.all(workers);

    const uniqueResults = fuseSearchResults(successfulGroups);

    return {
      results: uniqueResults,
      rawCount,
      failureCount: failures.length,
      failures,
    };
  }

  async fetchAdditionalPage(
    query: PlannedQuery,
    page: number,
    maxResultsPerQuery: number = 10,
  ): Promise<{ results: SearchResult[]; error?: string }> {
    try {
      const hits = await this.searchOnce({ ...query, page }, maxResultsPerQuery);
      return { results: hits };
    } catch (err: any) {
      return { results: [], error: String(err?.message || err) };
    }
  }
}
