/**
 * SearXNG Provider: Interfaces with a zero-cost, local/self-hosted SearXNG instance.
 *
 * Key-free default: only engines that need no credentials are requested.
 * Per-engine failures are preserved as diagnostics instead of failing the
 * whole search, empty results are distinct from provider failures, and
 * selective pagination (pageno) can recover hits absent from page one.
 */

import type { SearchProvider, SearchRequest, SearchResult } from '../types';
import { normalizeRawSearchResult } from './result_normalizer';

export interface SearXNGDiagnostics {
  httpStatus?: number;
  engines?: Record<string, string>;
  unresponsiveEngines?: string[];
  error?: string;
  page?: number;
}

/** Engines that require credentials stay off in the default configuration. */
export const KEYFREE_CREDENTIAL_ENGINES = ['google', 'bing', 'yandex', 'brave', 'mojeek'];

export class SearXNGProvider implements SearchProvider {
  private retryAfter = 0;
  public lastDiagnostics: SearXNGDiagnostics | null = null;
  constructor(
    private baseUrl: string = 'http://127.0.0.1:8080',
    private timeoutMs: number = 8000,
    private engines: string[] = ['duckduckgo', 'wikipedia', 'stackoverflow', 'github', 'arxiv', 'openstreetmap'],
    private disabledEngines: string[] = [...KEYFREE_CREDENTIAL_ENGINES],
  ) {}

  /** Verify which configured engines answer instead of assuming they work. */
  async checkEngineHealth(): Promise<{ working: string[]; failing: Record<string, string> }> {
    const working: string[] = [];
    const failing: Record<string, string> = {};
    await Promise.all(this.engines.map(async (engine) => {
      try {
        const url = new URL('/search', this.baseUrl);
        url.searchParams.set('q', 'health check');
        url.searchParams.set('format', 'json');
        url.searchParams.set('engines', engine);
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), Math.min(this.timeoutMs, 5000));
        try {
          const res = await fetch(url.toString(), {
            headers: { Accept: 'application/json' },
            signal: controller.signal,
            redirect: 'error',
          });
          if (!res.ok) failing[engine] = `HTTP ${res.status}`;
          else working.push(engine);
        } finally {
          clearTimeout(timer);
        }
      } catch (err: any) {
        failing[engine] = err?.name === 'AbortError' ? 'timeout' : String(err?.message || err);
      }
    }));
    return { working, failing };
  }

  async search(request: SearchRequest): Promise<SearchResult[]> {
    if (Date.now() < this.retryAfter) throw new Error('SearXNG Retry-After cooldown active');
    const page = Math.max(1, Math.min(5, Math.floor(request.page ?? 1) || 1));
    const url = new URL('/search', this.baseUrl);
    url.searchParams.set('q', request.query);
    url.searchParams.set('format', 'json');
    url.searchParams.set('pageno', String(page));
    if (this.engines.length > 0) url.searchParams.set('engines', this.engines.join(','));
    if (this.disabledEngines.length > 0) url.searchParams.set('disabled_engines', this.disabledEngines.join(','));
    if (request.language) url.searchParams.set('language', request.language);
    if (request.category) url.searchParams.set('categories', request.category);

    if (request.freshness && request.freshness !== 'any' && request.freshness !== 'realtime') {
      url.searchParams.set('time_range', request.freshness === 'week' ? 'month' : request.freshness);
    }

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const res = await fetch(url.toString(), {
        headers: { Accept: 'application/json' },
        signal: controller.signal,
        redirect: 'error',
      });

      if (!res.ok) {
        if (res.status === 429 || res.status === 503) {
          const value = res.headers.get('Retry-After') || '';
          const deadline = /^\d+$/.test(value) ? Date.now() + Number(value) * 1000 : Date.parse(value);
          this.retryAfter = Math.max(Date.now() + 30000, Number.isFinite(deadline) ? deadline : 0);
        }
        this.lastDiagnostics = { httpStatus: res.status, page, error: `HTTP ${res.status}` };
        throw new Error(`SearXNG returned HTTP ${res.status}: ${res.statusText}`);
      }

      const data = await res.json();
      clearTimeout(timer);
      const engines = data?.engines && typeof data.engines === 'object' ? data.engines : undefined;
      const unresponsive = Array.isArray(data?.unresponsive_engines)
        ? data.unresponsive_engines.map((e: unknown) => String(e)) : [];
      const engineFailures: Record<string, string> = {};
      if (engines) {
        for (const [name, info] of Object.entries(engines as Record<string, any>)) {
          const failed = info && typeof info === 'object' && ('error' in info || (typeof (info as any).timed_out !== 'undefined' && (info as any).timed_out));
          if (failed) engineFailures[name] = String((info as any).error || 'engine failed');
        }
      }
      this.lastDiagnostics = { httpStatus: res.status, engines: engineFailures, unresponsiveEngines: unresponsive, page };
      if (!data || !Array.isArray(data.results)) {
        // Empty result set is data, not a failure; diagnostics stay available.
        return [];
      }

      const limit = request.maxResults ?? 10;
      const results: SearchResult[] = [];

      for (let i = 0; i < Math.min(data.results.length, limit); i++) {
        results.push(normalizeRawSearchResult(data.results[i], request.query, i + 1, page));
      }

      return results;
    } catch (err: any) {
      clearTimeout(timer);
      if (err.name === 'AbortError') {
        this.lastDiagnostics = { page, error: 'timeout' };
        throw new Error(`SearXNG request timed out after ${this.timeoutMs}ms`);
      }
      if (!this.lastDiagnostics) this.lastDiagnostics = { page, error: String(err?.message || err) };
      throw err;
    }
  }
}
