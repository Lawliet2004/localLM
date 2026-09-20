/**
 * SearXNG Provider: Interfaces with a zero-cost, local/self-hosted SearXNG instance.
 *
 * Key-free default: only engines that need no credentials are requested.
 * Per-engine failures are preserved as diagnostics instead of failing the
 * whole search, empty results are distinct from provider failures, and
 * selective pagination (pageno) can recover hits absent from page one.
 */

import type { SearchProvider, SearchRequest, SearchResult, SearchMeta, RetrievedDocument } from '../types';
import { emptySearchMeta, mergeSearchMeta } from '../types';
import { normalizeRawSearchResult } from './result_normalizer';
import { computeContentHash } from '../extraction/main_content';

export interface SearXNGDiagnostics {
  httpStatus?: number;
  engines?: Record<string, string>;
  unresponsiveEngines?: string[];
  error?: string;
  page?: number;
}

/**
 * Normalize SearXNG's version-varying meta fields into SearchMeta. Answers may
 * be plain strings or {answer,url} objects; infoboxes expose `infobox`/`content`
 * with optional `urls`. Empty entries are dropped.
 */
export function normalizeSearxMeta(data: any): SearchMeta {
  const meta = emptySearchMeta();
  if (Array.isArray(data?.answers)) {
    meta.answers = data.answers
      .map((a: any) => (typeof a === 'string' ? { answer: a } : { answer: String(a?.answer ?? a?.content ?? ''), url: a?.url ? String(a.url) : undefined }))
      .filter((a: { answer: string }) => a.answer.trim().length > 0)
      .slice(0, 5);
  }
  if (Array.isArray(data?.infoboxes)) {
    meta.infoboxes = data.infoboxes
      .map((ib: any) => ({
        title: String(ib?.infobox ?? ib?.title ?? ''),
        content: String(ib?.content ?? ''),
        url: ib?.urls?.[0]?.url ? String(ib.urls[0].url) : undefined,
      }))
      .filter((ib: { title: string; content: string }) => ib.title.trim().length > 0 || ib.content.trim().length > 0)
      .slice(0, 3);
  }
  if (Array.isArray(data?.corrections)) {
    meta.corrections = data.corrections
      .map((c: any) => (typeof c === 'string' ? c : String(c?.title ?? '')))
      .filter((c: string) => c.trim().length > 0)
      .slice(0, 3);
  }
  if (Array.isArray(data?.suggestions)) {
    meta.suggestions = data.suggestions.filter((s: unknown) => typeof s === 'string' && s.trim().length > 0).slice(0, 8);
  }
  return meta;
}

/** Engines that require credentials stay off in the default configuration. */
export const KEYFREE_CREDENTIAL_ENGINES = ['google', 'bing', 'yandex', 'brave', 'mojeek'];

const resultMeta = new WeakMap<SearchResult[], SearchMeta>();

/** Meta attached to a `search()` return value so concurrent queries cannot clobber each other. */
export function metaForResults(results: SearchResult[]): SearchMeta | undefined {
  return resultMeta.get(results);
}

export function parseSearxngBaseUrls(baseUrl: string): string[] {
  const urls = baseUrl.split(/[,;]/).map((s) => s.trim()).filter(Boolean);
  return urls.length > 0 ? urls : ['http://127.0.0.1:8080'];
}

/** Instant answers and infoboxes become evidence documents for small local models. */
export function documentsFromSearchMeta(meta: SearchMeta, retrievedAt: string = new Date().toISOString()): RetrievedDocument[] {
  const docs: RetrievedDocument[] = [];
  meta.answers.forEach((answer, i) => {
    const url = usableMetaUrl(answer.url) || `https://answer.invalid/searxng/${i + 1}`;
    const text = answer.answer.trim();
    if (!text) return;
    docs.push({
      id: `searx-answer-${i + 1}`,
      url,
      domain: hostnameOf(url),
      title: 'Instant answer',
      text,
      contentHash: computeContentHash(text),
      retrievedAt,
      searchResultIds: [],
      metadata: { extractionMethod: 'searxng_answer' },
    });
  });
  meta.infoboxes.forEach((box, i) => {
    const url = usableMetaUrl(box.url) || `https://infobox.invalid/searxng/${i + 1}`;
    const text = [box.title, box.content].filter(Boolean).join('\n\n').trim();
    if (!text) return;
    docs.push({
      id: `searx-infobox-${i + 1}`,
      url,
      domain: hostnameOf(url),
      title: box.title || 'Infobox',
      text,
      contentHash: computeContentHash(text),
      retrievedAt,
      searchResultIds: [],
      metadata: { extractionMethod: 'searxng_infobox' },
    });
  });
  return docs;
}

function usableMetaUrl(url?: string): string | undefined {
  if (!url) return undefined;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return undefined;
    return parsed.href;
  } catch {
    return undefined;
  }
}

function hostnameOf(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return 'searxng';
  }
}

export class SearXNGProvider implements SearchProvider {
  private retryAfter = 0;
  public lastDiagnostics: SearXNGDiagnostics | null = null;
  /** Instant answers / infoboxes / corrections / suggestions from the last successful search. */
  public lastMeta: SearchMeta | null = null;
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
        const url = new URL('/search', parseSearxngBaseUrls(this.baseUrl)[0]);
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
    const bases = parseSearxngBaseUrls(this.baseUrl);
    const deadline = Date.now() + this.timeoutMs;
    let lastError: unknown;
    let cooldownUntil = 0;
    for (let i = 0; i < bases.length; i++) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break;
      try {
        const hits = await this.searchAgainst(bases[i], request, remaining);
        return hits;
      } catch (err: any) {
        lastError = err;
        if (typeof err?.cooldownUntil === 'number') cooldownUntil = Math.max(cooldownUntil, err.cooldownUntil);
        if (bases.length === 1) {
          if (cooldownUntil) this.retryAfter = cooldownUntil;
          throw err;
        }
      }
    }
    if (cooldownUntil) this.retryAfter = cooldownUntil;
    throw lastError instanceof Error ? lastError : new Error('SearXNG request failed');
  }

  private async searchAgainst(baseUrl: string, request: SearchRequest, timeoutMs: number): Promise<SearchResult[]> {
    const page = Math.max(1, Math.min(5, Math.floor(request.page ?? 1) || 1));
    const url = new URL('/search', baseUrl);
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
    const timer = setTimeout(() => controller.abort(), Math.max(50, timeoutMs));

    try {
      const res = await fetch(url.toString(), {
        headers: { Accept: 'application/json' },
        signal: controller.signal,
        redirect: 'error',
      });

      if (!res.ok) {
        this.lastDiagnostics = { httpStatus: res.status, page, error: `HTTP ${res.status}` };
        const error: Error & { cooldownUntil?: number } = new Error(`SearXNG returned HTTP ${res.status}: ${res.statusText}`);
        if (res.status === 429 || res.status === 503) {
          const value = res.headers.get('Retry-After') || '';
          const parsed = /^\d+$/.test(value) ? Date.now() + Number(value) * 1000 : Date.parse(value);
          error.cooldownUntil = Math.max(Date.now() + 30000, Number.isFinite(parsed) ? parsed : 0);
        }
        throw error;
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
      const meta = normalizeSearxMeta(data);
      this.lastMeta = this.lastMeta ? mergeSearchMeta(this.lastMeta, meta) : meta;
      if (!data || !Array.isArray(data.results)) {
        const empty: SearchResult[] = [];
        resultMeta.set(empty, meta);
        return empty;
      }

      const limit = request.maxResults ?? 10;
      const results: SearchResult[] = [];

      for (let i = 0; i < Math.min(data.results.length, limit); i++) {
        results.push(normalizeRawSearchResult(data.results[i], request.query, i + 1, page));
      }
      resultMeta.set(results, meta);
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
