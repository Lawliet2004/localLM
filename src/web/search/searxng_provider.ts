/**
 * SearXNG Provider: Interfaces with a zero-cost, local/self-hosted SearXNG instance.
 *
 * Uses the instance's configured engines unless explicitly overridden.
 * Per-engine failures are preserved as diagnostics instead of failing the
 * whole search, empty results are distinct from provider failures, and
 * selective pagination (pageno) can recover hits absent from page one.
 */

import type { SearchProvider, SearchRequest, SearchResult, SearchMeta, RetrievedDocument } from '../types';
import { emptySearchMeta } from '../types';
import { normalizeRawSearchResult } from './result_normalizer';
import { computeContentHash } from '../extraction/main_content';

export interface SearXNGDiagnostics {
  baseUrl?: string;
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

const resultMeta = new WeakMap<SearchResult[], SearchMeta>();

/** Meta attached to a `search()` return value so concurrent queries cannot clobber each other. */
export function metaForResults(results: SearchResult[]): SearchMeta | undefined {
  return resultMeta.get(results);
}

export function parseSearxngBaseUrls(baseUrl: string): string[] {
  const urls = baseUrl.split(/[,;]/).map((s) => s.trim()).filter(Boolean);
  return [...new Set((urls.length ? urls : ['http://127.0.0.1:8080']).map((value) => {
    let url: URL;
    try { url = new URL(value); }
    catch { throw new Error('SearXNG base URL must be an absolute HTTP or HTTPS URL.'); }
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
      throw new Error('SearXNG base URL must use HTTP or HTTPS without credentials, a query, or a fragment.');
    }
    // Accept a pasted /search endpoint and preserve reverse-proxy prefixes.
    url.pathname = url.pathname.replace(/\/+$/, '').replace(/\/search$/, '') + '/';
    return url.href.replace(/\/$/, '');
  }))];
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

export class SearXNGError extends Error {
  constructor(message: string, public diagnostics: SearXNGDiagnostics, public retryable = true) {
    super(message);
    this.name = 'SearXNGError';
  }
}

function engineFailures(data: any): Record<string, string> {
  const failures: Record<string, string> = {};
  if (data.engines && !Array.isArray(data.engines) && typeof data.engines === 'object') {
    for (const [name, info] of Object.entries(data.engines as Record<string, any>)) {
      if (info?.error || info?.timed_out) failures[name] = String(info.error || 'timeout');
    }
  }
  for (const entry of Array.isArray(data.unresponsive_engines) ? data.unresponsive_engines : []) {
    // SearXNG emits [engine, reason] tuples, including suspended engines.
    if (Array.isArray(entry) && typeof entry[0] === 'string') {
      failures[entry[0]] = String(entry[1] || 'unresponsive');
    } else if (typeof entry === 'string') {
      failures[entry] = 'unresponsive';
    }
  }
  return failures;
}

export class SearXNGProvider implements SearchProvider {
  private retryAfter = new Map<string, number>();
  public lastDiagnostics: SearXNGDiagnostics | null = null;
  /** Metadata from the last completed search; per-query metadata travels with results. */
  public lastMeta: SearchMeta | null = null;
  constructor(
    private baseUrl: string = 'http://127.0.0.1:8080',
    private timeoutMs: number = 15000,
    private engines: string[] = [],
    private disabledEngines: string[] = [],
  ) {}

  /** Probe the real JSON search API, including upstream engine failures. */
  async checkHealth(): Promise<{ search: string; searchError?: string; searxngDiagnostics?: SearXNGDiagnostics }> {
    try {
      const hits = await this.search({ query: 'SearXNG', maxResults: 1 });
      const degraded = Object.keys(this.lastDiagnostics?.engines || {}).length > 0;
      const hasEvidence = hits.length || this.lastMeta?.answers.length || this.lastMeta?.infoboxes.length;
      return {
        search: degraded ? 'degraded' : hasEvidence ? 'healthy' : 'empty',
        ...(!hasEvidence ? { searchError: 'SearXNG JSON API responded, but the probe returned no results. Check the enabled engines.' } : {}),
        searxngDiagnostics: this.lastDiagnostics ?? undefined,
      };
    } catch (error) {
      return {
        search: this.lastDiagnostics?.httpStatus ? 'error' : 'unreachable',
        searchError: error instanceof Error ? error.message : String(error),
        searxngDiagnostics: this.lastDiagnostics ?? undefined,
      };
    }
  }

  /** HTTP 200 alone does not establish that an engine actually answered. */
  async checkEngineHealth(): Promise<{ working: string[]; failing: Record<string, string> }> {
    const working: string[] = [];
    const failing: Record<string, string> = {};
    const queue = [...this.engines];
    await Promise.all(Array.from({ length: Math.min(4, queue.length) }, async () => {
      while (queue.length) {
        const engine = queue.shift()!;
        try {
          const probe = new SearXNGProvider(this.baseUrl, this.timeoutMs, [engine]);
          const hits = await probe.search({ query: 'SearXNG', maxResults: 1 });
          const reason = probe.lastDiagnostics?.engines?.[engine];
          if (reason) failing[engine] = reason;
          else if (hits.length || probe.lastMeta?.answers.length || probe.lastMeta?.infoboxes.length) working.push(engine);
          else failing[engine] = 'No results returned for the probe';
        } catch (error) {
          failing[engine] = error instanceof Error ? error.message : String(error);
        }
      }
    }));
    return { working, failing };
  }

  async search(request: SearchRequest): Promise<SearchResult[]> {
    this.lastMeta = null;
    this.lastDiagnostics = null;
    if (typeof request.query !== 'string' || !request.query.trim()) {
      throw new SearXNGError('SearXNG requires a nonempty search query.', {}, false);
    }
    let bases: string[];
    try { bases = parseSearxngBaseUrls(this.baseUrl); }
    catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.lastDiagnostics = { error: message };
      throw new SearXNGError(message, this.lastDiagnostics, false);
    }
    const deadline = Date.now() + this.timeoutMs;
    let lastError: SearXNGError | undefined;
    const available = bases.filter(base => (this.retryAfter.get(base) || 0) <= Date.now());
    if (!available.length) {
      this.lastDiagnostics = { error: 'SearXNG Retry-After cooldown active for all configured instances.' };
      throw new SearXNGError(this.lastDiagnostics.error!, this.lastDiagnostics, false);
    }
    for (let i = 0; i < available.length; i++) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break;
      try {
        // Reserve time for the remaining instances if one server hangs.
        const outcome = await this.searchAgainst(available[i], request, remaining / (available.length - i));
        this.lastDiagnostics = outcome.diagnostics;
        this.lastMeta = outcome.meta;
        resultMeta.set(outcome.results, outcome.meta);
        return outcome.results;
      } catch (error) {
        lastError = error as SearXNGError;
        this.lastDiagnostics = lastError.diagnostics;
      }
    }
    throw lastError || new SearXNGError('SearXNG request timed out.', { error: 'timeout' });
  }

  private async searchAgainst(baseUrl: string, request: SearchRequest, timeoutMs: number) {
    const page = Math.max(1, Math.min(5, Math.floor(request.page ?? 1) || 1));
    const diagnostics: SearXNGDiagnostics = { baseUrl, page };
    const url = new URL(`${baseUrl}/search`);
    url.searchParams.set('q', request.query.trim());
    url.searchParams.set('format', 'json');
    url.searchParams.set('pageno', String(page));
    const enabled = this.engines.filter(engine => !this.disabledEngines.includes(engine));
    if (this.engines.length && !enabled.length) {
      throw new SearXNGError('All requested SearXNG engines are disabled.', diagnostics, false);
    }
    if (enabled.length) url.searchParams.set('engines', enabled.join(','));
    if (this.disabledEngines.length) url.searchParams.set('disabled_engines', this.disabledEngines.join(','));
    if (request.language) url.searchParams.set('language', request.language);
    if (request.category) url.searchParams.set('categories', request.category);
    if (request.freshness && request.freshness !== 'any') {
      // The API supports day/month/year; week uses the nearest wider window.
      url.searchParams.set('time_range', request.freshness === 'week' ? 'month' : request.freshness === 'realtime' ? 'day' : request.freshness);
    }
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), Math.max(1, Math.floor(timeoutMs)));
    try {
      const res = await fetch(url.toString(), {
        headers: { Accept: 'application/json' }, signal: controller.signal, redirect: 'error',
      });
      diagnostics.httpStatus = res.status;
      if (!res.ok) {
        let message = `SearXNG returned HTTP ${res.status}.`;
        if (res.status === 403) message += ' Enable json under search.formats in SearXNG settings.yml and check access restrictions.';
        if (res.status === 429 || res.status === 503) {
          const value = res.headers.get('Retry-After')?.trim();
          const parsed = value && /^\d+$/.test(value) ? Date.now() + Number(value) * 1000 : Date.parse(value || '');
          this.retryAfter.set(baseUrl, Number.isFinite(parsed) ? Math.max(Date.now(), parsed) : Date.now() + 30000);
          message += ' Retry-After cooldown applied to this instance.';
        }
        throw new SearXNGError(message, diagnostics, res.status >= 500 && res.status !== 503);
      }
      let data: any;
      try { data = await res.json(); }
      catch (error) {
        if (controller.signal.aborted) throw error;
        throw new SearXNGError('SearXNG did not return valid JSON. Enable json under search.formats in settings.yml and check the base URL.', diagnostics, false);
      }
      if (!data || !Array.isArray(data.results) || data.error) {
        throw new SearXNGError(`SearXNG returned an invalid search response${data?.error ? `: ${String(data.error).slice(0, 300)}` : ': missing results array'}.`, diagnostics, false);
      }
      diagnostics.engines = engineFailures(data);
      diagnostics.unresponsiveEngines = Object.keys(diagnostics.engines);
      const meta = normalizeSearxMeta(data);
      const limit = Math.max(1, Math.min(50, Math.floor(request.maxResults ?? 10) || 10));
      const results: SearchResult[] = [];
      for (let i = 0; i < data.results.length && results.length < limit; i++) {
        const raw = data.results[i];
        // Ignore non-web result types and malformed entries without losing good hits.
        if (!raw || typeof raw !== 'object' || !usableMetaUrl(raw.url)) continue;
        results.push(normalizeRawSearchResult(raw, request.query, i + 1, page));
      }
      if (!results.length && !meta.answers.length && !meta.infoboxes.length && diagnostics.unresponsiveEngines.length) {
        throw new SearXNGError(`SearXNG returned no usable results; engine failures: ${Object.entries(diagnostics.engines).map(([name, reason]) => `${name}: ${reason}`).join('; ')}`, diagnostics);
      }
      return { results, meta, diagnostics };
    } catch (error) {
      if (controller.signal.aborted) {
        throw new SearXNGError(`SearXNG request timed out after ${this.timeoutMs}ms (instance budget ${Math.floor(timeoutMs)}ms).`, { ...diagnostics, error: 'timeout' });
      }
      const failure = error instanceof SearXNGError ? error : new SearXNGError(`Could not reach SearXNG at ${baseUrl}: ${error instanceof Error ? error.message : String(error)}`, diagnostics);
      failure.diagnostics.error = failure.message;
      throw failure;
    } finally {
      clearTimeout(timer);
    }
  }
}
