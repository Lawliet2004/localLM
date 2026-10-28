/** Search and document retrieval. The agent owns planning and synthesis. */
import type { SearchProvider, RetrievedDocument, ResearchSession, ResearchMode, FreshnessWindow } from './types';
import { ttlForFreshness } from './cache/search_cache';
import { DocumentCache } from './cache/document_cache';
import { mergeSearchMeta } from './types';
import { createConfig } from './config/defaults';
import type { WebSearchConfig } from './config/schema';
import { SearchService } from './search/search_service';
import { SearXNGProvider } from './search/searxng_provider';
import { GoogleSearchProvider } from './search/google_provider';
import { HttpFetcher } from './fetch/http_fetcher';
import { fetchPageCascade, type CascadeOutcome, type CascadeOptions } from './fetch/cascade';
import { DomainStatsStore } from './fetch/domain_stats';
import { extractPdfText } from './documents/document_store';
import { extractStructuredContent } from './extraction/structured_content';
import { SourceManager } from './evidence/source_registry';
import { TraceCollector } from './observability/trace';
import { InMemoryStorageAdapter, type StorageAdapter } from './cache/sqlite';
export * from './types';
export * from './config/schema';
export * from './config/defaults';
export { formatDiagnosticReport } from './observability/metrics';
export interface ResearchProgress { stage: 'route' | 'plan' | 'search' | 'fetch' | 'evidence' | 'answer' | 'verify'; message: string; done?: number; total?: number }
export interface EngineDependencies {
  searchProvider?: SearchProvider;
  fetcher?: Pick<HttpFetcher, 'fetch'>; jsRender?: import('./fetch/cascade').JsRenderer;
  storage?: StorageAdapter; config?: Partial<WebSearchConfig>; onProgress?: (progress: ResearchProgress) => void;
}
export class WebSearchEngine {
  public config: WebSearchConfig;
  private searchProvider: SearchProvider;
  private fallbackProvider?: SearchProvider;
  private searchStorage?: StorageAdapter;
  private fetcher: Pick<HttpFetcher, 'fetch'>;
  private jsRender?: import('./fetch/cascade').JsRenderer;
  private domainStats?: DomainStatsStore;
  private inflightFetches = new Map<string, Promise<CascadeOutcome>>();
  private onProgress?: (progress: ResearchProgress) => void;
  constructor(deps: EngineDependencies = {}) {
    this.config = createConfig(deps.config);
    const google = this.config.googleApiKey && this.config.googleCxId
      ? new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId) : undefined;
    const searxng = new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines);
    this.searchProvider = deps.searchProvider || (this.config.searchProvider === 'google' && google ? google : searxng);
    this.fallbackProvider = !deps.searchProvider && this.config.searchFallback?.enabled
      ? (this.searchProvider === google ? searxng : google) : undefined;
    this.searchStorage = deps.storage;
    this.fetcher = deps.fetcher || new HttpFetcher(this.config.fetch.globalConcurrency, this.config.fetch.perDomainConcurrency);
    this.jsRender = this.config.fetch.jsRenderFallback ? deps.jsRender : undefined;
    this.domainStats = this.config.fetch.domainLearning ? new DomainStatsStore(deps.storage ?? new InMemoryStorageAdapter()) : undefined;
    this.onProgress = deps.onProgress;
  }
  private fetchCascade(url: string, options: CascadeOptions): Promise<CascadeOutcome> {
    const pending = this.inflightFetches.get(url);
    if (pending) return pending;
    const promise = fetchPageCascade(url, this.fetcher, options)
      .finally(() => { if (this.inflightFetches.get(url) === promise) this.inflightFetches.delete(url); });
    this.inflightFetches.set(url, promise);
    return promise;
  }

  private async claimGoogleQuota(queryCount = 1): Promise<boolean> {
    const limit = this.config.searchFallback?.googleDailyLimit ?? 90;
    if (!this.searchStorage) return true;
    const day = new Date().toISOString().split('T')[0];
    const key = `google_search_count_${day}`;
    const used = (await this.searchStorage.get<number>('quota', key)) ?? 0;
    if (used >= limit) return false;
    await this.searchStorage.set('quota', key, used + queryCount, 90000);
    return true;
  }

  async getHealthStatus() {
    const health = this.config.searchProvider === 'google' && (!this.config.googleApiKey || !this.config.googleCxId)
      ? { search:'not_configured', searchError:'Configure a Google API key and Custom Search Engine ID.' }
      : this.searchProvider instanceof SearXNGProvider ? await this.searchProvider.checkHealth() : { search:'ready' };
    return { ...health, database:this.config.cache.enabled ? 'configured' : 'disabled',
      embeddingModel:'disabled', reranker:'disabled', llm:'agent_owned',
      google:this.config.googleApiKey && this.config.googleCxId ? 'configured' : 'not_configured' };
  }
  /** Direct search for agent-controlled research; no nested planning/synthesis. */
  async searchQueries(queries: string[], filters: {freshness?: FreshnessWindow; language?: string; category?: string} = {}) {
    if (!Array.isArray(queries) || queries.length < 1 || queries.length > 4 ||
        queries.some(q => typeof q !== 'string' || !q.trim() || new TextEncoder().encode(q).length > 8000)) {
      throw new Error('search requires 1-4 nonempty queries, at most 8000 bytes each');
    }
    const planned = [...new Set(queries.map(q => q.trim()))].map(query => ({ query, purpose: 'agent research', freshness: filters.freshness || 'any' as FreshnessWindow, language:filters.language, category:filters.category }));
    const cacheKey = JSON.stringify([this.config.searchProvider, this.config.searxngBaseUrl, planned]);
    const cached = this.config.cache.enabled ? await this.searchStorage?.get<Awaited<ReturnType<SearchService['executeSearches']>>>('agent_search',cacheKey) : null;
    if (cached) return cached;
    if (this.searchProvider instanceof GoogleSearchProvider && !(await this.claimGoogleQuota(planned.length))) {
      throw new Error('Google daily quota exhausted');
    }
    const options = { maxConcurrentQueries: this.config.maxConcurrentQueries, retries: this.config.searchRetries, retryDelayMs: this.config.searchRetryDelayMs };
    const result = await new SearchService(this.searchProvider, options).executeSearches(planned, 10);
    if (result.results.length || result.meta.answers.length || result.meta.infoboxes.length || !this.fallbackProvider) {
      if (this.config.cache.enabled && !result.failureCount) await this.searchStorage?.set('agent_search',cacheKey,result,
        ttlForFreshness(filters.freshness || 'any', this.config.cache.searchTtlSeconds.default));
      return result;
    }
    if (this.fallbackProvider instanceof GoogleSearchProvider && !(await this.claimGoogleQuota(planned.length))) return result;
    const fallback = await new SearchService(this.fallbackProvider, options).executeSearches(planned, 10);
    return {
      ...fallback,
      failureCount: result.failureCount + fallback.failureCount,
      failures: [...result.failures, ...fallback.failures],
      meta: mergeSearchMeta(result.meta, fallback.meta),
    };
  }

  async fetchUrl(rawUrl: string): Promise<RetrievedDocument> {
    const cache = this.config.cache.enabled ? new DocumentCache(this.searchStorage) : null;
    const cached = await cache?.get(rawUrl);
    if (cached) return cached;
    const document = await this.retrieveUrl(rawUrl);
    await cache?.set(document);
    return document;
  }
  private async retrieveUrl(rawUrl: string): Promise<RetrievedDocument> {
    const outcome = await this.fetchCascade(rawUrl, {
      timeoutSeconds: this.config.fetch.timeoutSeconds,
      maxBytes: this.config.fetch.maxBytes,
      userAgent: this.config.fetch.userAgent,
      waybackFallback: this.config.fetch.waybackFallback,
      domainStats: this.domainStats,
      jsRender: this.jsRender,
    });
    const res = outcome.raw;
    const mime = (res?.mimeType || '').split(';')[0].trim().toLowerCase();
    if (res?.success && typeof res.body === 'string' && (['application/json', 'text/csv', 'application/csv', 'text/tab-separated-values'].includes(mime) || mime.endsWith('+json'))) {
      const structured = extractStructuredContent(res.body, mime);
      const url = res.finalUrl || rawUrl;
      return { id: `data-${crypto.randomUUID()}`, url, domain: new URL(url).hostname, title: new URL(url).pathname.split('/').pop() || 'Structured document',
        text: structured.text, retrievedAt: new Date().toISOString(), searchResultIds: [], metadata: { extractionMethod: structured.extractionMethod, rowCount: structured.rows } };
    }
    if (res?.success && res.body && (mime === 'application/pdf' || /\.pdf(\?|#|$)/i.test(res.finalUrl || rawUrl))) {
      const latin1 = res.body || '';
      const bytes = Uint8Array.from(latin1, (ch) => ch.charCodeAt(0) & 0xff);
      const pdf = extractPdfText(bytes);
      switch (pdf.status) {
        case 'text_extracted':
          break;
        case 'no_extractable_text':
          throw new Error('This PDF has no extractable text.');
        case 'password_required':
          throw new Error('This PDF is password-protected and cannot be extracted locally.');
        case 'invalid_document':
          throw new Error('The response is not a valid PDF.');
        case 'limit_exceeded':
          throw new Error('PDF exceeds the local extraction size limit.');
        default: {
          const unexpected: never = pdf.status;
          throw new Error(`PDF extraction failed (${String(unexpected)}).`);
        }
      }
      return {
        id: `pdf-${crypto.randomUUID()}`,
        url: res.finalUrl || rawUrl,
        domain: new URL(res.finalUrl || rawUrl).hostname,
        title: rawUrl.split('/').pop() || 'PDF document',
        text: pdf.text,
        pages: pdf.pages,
        retrievedAt: new Date().toISOString(),
        searchResultIds: [],
        metadata: { extractionMethod: 'pdf_text', pageCount: pdf.pages.length },
      };
    }
    if (outcome.document) {
      return {
        id: `fetch-${crypto.randomUUID()}`,
        url: outcome.document.finalUrl || rawUrl,
        domain: new URL(outcome.document.finalUrl || rawUrl).hostname,
        title: outcome.document.title || rawUrl,
        text: outcome.document.text,
        headings: outcome.document.headings,
        links: outcome.document.links,
        author: outcome.document.author,
        publishedAt: outcome.document.publishedAt,
        retrievedAt: new Date().toISOString(),
        searchResultIds: [],
        metadata: {
          extractionMethod: outcome.method === 'github_raw' ? 'github_raw'
            : outcome.method === 'js_render' ? 'js_render'
            : outcome.method === 'wayback' ? 'wayback' : 'main_content',
          archivedAt: outcome.document.archivedAt,
        },
      };
    }
    throw new Error(`Fetch failed: ${outcome.error || res?.error || 'unknown error'}`);
  }

  async research(question: string, options: { mode?: ResearchMode; currentDate?: string; freshness?: FreshnessWindow; language?: string; category?: string } = {}): Promise<ResearchSession> {
    if (!this.config.enabled) throw new Error('Web research is disabled');
    if (!question.trim() || new TextEncoder().encode(question).length > 8000) throw new Error('Question must contain 1–8000 bytes');
    const startedAt = new Date().toISOString();
    this.onProgress?.({stage:'search', message:'Searching public sources'});
    const outcome = await this.searchQueries([question], options);
    const sources = new SourceManager();
    const documents: RetrievedDocument[] = outcome.results.map((hit, index) => ({
      id:`search-${index + 1}`, url:hit.url, title:hit.title, domain:new URL(hit.url).hostname,
      text:hit.snippet || '', retrievedAt:new Date().toISOString(), searchResultIds:[hit.id],
      metadata:{extractionMethod:'snippet_only'},
    }));
    const snippets = documents.map(doc => `[${sources.registerDocument(doc)}] ${doc.title} — ${doc.url}\n${doc.text}`);
    const trace = new TraceCollector();
    trace.update({ queriesGenerated:1, searchResults:outcome.results.length, uniqueResults:documents.length,
      searchFailureCount:outcome.failureCount, queryFailures:outcome.failures,
      providerUsed:this.config.searchProvider, searxngDiagnostics:this.searchProvider instanceof SearXNGProvider ? this.searchProvider.lastDiagnostics ? { ...this.searchProvider.lastDiagnostics } : undefined : undefined, totalLatencyMs:Date.now()-Date.parse(startedAt) });
    return { id:`sess-${crypto.randomUUID()}`, question, normalizedQuery:question.trim(),
      route:{vertical:'GENERAL_WEB', confidence:1, requiresExternalData:true, requiresWebSearch:true, freshness:options.freshness || 'any'},
      queries:[{query:question, purpose:'agent search', freshness:options.freshness || 'any'}],
      results:outcome.results, searchMeta:outcome.meta, documents, chunks:[], retrievedChunks:[], rerankedChunks:[], evidence:[],
      sources:sources.getAllSources(), answer:snippets.join('\n\n'),
      tokenUsage:{systemBudget:0,questionBudget:0,evidenceBudget:0,safetyMargin:0,totalInputBudget:0,estimatedEvidenceTokens:0,finalPromptTokens:0},
      startedAt, completedAt:new Date().toISOString(), trace:trace.getTrace(),
      limitations:['Search snippets are unverified source content. Open relevant pages before making factual claims.'],
    };
  }
}
