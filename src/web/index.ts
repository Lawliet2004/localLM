/**
 * Zero-Cost, Local-First Web Search + Grounding Engine for Small LLMs
 *
 * Core architectural principle:
 * Search broadly outside the LLM context (inspecting dozens of results and tens of thousands
 * of source tokens), algorithmically filter, chunk, retrieve, rerank, and compress evidence,
 * and provide the final 5B-10B parameter LLM with only ~1.5K-3K tokens of strictly grounded evidence.
 */

import type {
  SearchProvider,
  LLMProvider,
  ResearchSession,
  ResearchMode,
  RetrievedDocument,
  EvidenceChunk,
  ExtractedFact,
  EvidenceClaim,
  TokenStats,
  SearchMeta,
  SearchResult,
  FreshnessWindow,
  PlannedQuery,
} from './types';
import { emptySearchMeta, mergeSearchMeta } from './types';
import { createConfig, researchBudget } from './config/defaults';
import type { WebSearchConfig } from './config/schema';
import { fuseSearchResults } from './search/result_fusion';
import { extractPdfText } from './documents/document_store';
import { normalizeRequest } from './router/request_normalizer';
import { routeRequest } from './router/intent_router';
import { QueryPlanner } from './planning/query_planner';
import { SearchService } from './search/search_service';
import { SearXNGProvider, documentsFromSearchMeta } from './search/searxng_provider';
import { GoogleSearchProvider } from './search/google_provider';
import { rankSearchResults } from './ranking/search_ranker';
import { selectPagesToFetch } from './fetch/fetch_policy';
import { HttpFetcher } from './fetch/http_fetcher';
import { fetchPageCascade } from './fetch/cascade';
import { FetchFailure } from './fetch/failure';
import { DomainStatsStore, orderByDomainHealth } from './fetch/domain_stats';
import { fallbackSnippetExtraction } from './extraction/extraction_fallback';
import { chunkDocument } from './chunking/semantic_chunker';
import { retrieveChunks } from './retrieval/hybrid_retriever';
import { RerankerService } from './reranking/reranker';
import { SourceManager } from './evidence/source_registry';
import { EvidenceExtractor } from './evidence/extractor';
import { deduplicateFacts } from './evidence/deduplicator';
import { detectEvidenceConflicts } from './evidence/conflict_detector';
import { buildGroundedContext, buildModelBudgetedContext } from './context/context_builder';
import { AnswerGenerator } from './generation/answer_generator';
import { extractAtomicClaims } from './verification/claim_extractor';
import { ClaimVerifier } from './verification/claim_verifier';
import { evaluateResearchRetry, isRepeatedQuery } from './verification/research_retry';
import { initResearchState, recordRound, classifyRequirements } from './planning/research_state';
import { WeatherProvider } from './verticals/weather/weather_provider';
import { CurrencyProvider } from './verticals/currency/currency_provider';
import { TimeProvider } from './verticals/time/time_provider';
import { SearchCache } from './cache/search_cache';
import { ttlForFreshness } from './cache/search_cache';
import { DocumentCache } from './cache/document_cache';
import { TraceCollector } from './observability/trace';
import { defaultTokenCounter } from './chunking/tokenizer';
import { InMemoryStorageAdapter, type StorageAdapter } from './cache/sqlite';
import { coverageMatrix } from './planning/coverage';
import { isSafeUrl } from './security/ssrf_guard';
import { runStagedResearch } from './staged_research';
import { localDateString } from './planning/fallback_planner';

export * from './types';
export * from './config/schema';
export * from './config/defaults';
export { formatDiagnosticReport } from './observability/metrics';
export { FetchFailure } from './fetch/failure';

export interface EngineDependencies {
  searchProvider?: SearchProvider;
  llmProvider?: LLMProvider;
  fetcher?: Pick<HttpFetcher, 'fetch'>;
  jsRender?: import('./fetch/cascade').JsRenderer;
  storage?: StorageAdapter;
  config?: Partial<WebSearchConfig>;
}

export class WebSearchEngine {
  public config: WebSearchConfig;
  private searchProvider: SearchProvider;
  private fallbackProvider?: SearchProvider;
  private searchStorage: StorageAdapter;
  private llmProvider?: LLMProvider;
  private reranker: RerankerService;
  private fetcher: Pick<HttpFetcher, 'fetch'>;
  private jsRender?: import('./fetch/cascade').JsRenderer;
  private searchCache: SearchCache;
  private documentCache: DocumentCache;
  private domainStats?: DomainStatsStore;
  private weatherProvider: WeatherProvider;
  private currencyProvider: CurrencyProvider;
  private timeProvider: TimeProvider;
  private queryPlanner: QueryPlanner;
  private evidenceExtractor: EvidenceExtractor;
  private answerGenerator: AnswerGenerator;
  private claimVerifier: ClaimVerifier;

  constructor(deps: EngineDependencies = {}) {
    this.config = createConfig(deps.config);
    this.searchStorage = deps.storage ?? new InMemoryStorageAdapter();
    if (this.config.searchProvider === 'google' && this.config.googleApiKey && this.config.googleCxId) {
      this.searchProvider = deps.searchProvider || new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId, 8000, () => this.claimGoogleQuota());
      this.fallbackProvider = (!deps.searchProvider && this.config.searchFallback?.enabled)
        ? new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines, this.searchStorage) : undefined;
    } else {
      this.searchProvider = deps.searchProvider || new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines, this.searchStorage);
      this.fallbackProvider = (!deps.searchProvider && this.config.searchFallback?.enabled && this.config.googleApiKey && this.config.googleCxId)
        ? new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId, 8000, () => this.claimGoogleQuota()) : undefined;
    }
    this.llmProvider = deps.llmProvider;
    this.reranker = new RerankerService(this.config.reranking.enabled, this.config.reranking.recencyWeight ?? 0);
    this.fetcher =
      deps.fetcher ||
      new HttpFetcher(
        this.config.fetch.globalConcurrency,
        this.config.fetch.perDomainConcurrency
      );
    this.jsRender = this.config.fetch.jsRenderFallback ? deps.jsRender : undefined;
    this.searchCache = new SearchCache(this.searchStorage);
    this.documentCache = new DocumentCache(this.searchStorage);
    this.domainStats = this.config.fetch.domainLearning
      ? new DomainStatsStore(this.searchStorage)
      : undefined;
    this.weatherProvider = new WeatherProvider();
    this.currencyProvider = new CurrencyProvider();
    this.timeProvider = new TimeProvider();
    this.queryPlanner = new QueryPlanner(this.llmProvider);
    this.evidenceExtractor = new EvidenceExtractor(this.config.extractEvidenceWithModel ? this.llmProvider : undefined);
    this.answerGenerator = new AnswerGenerator(this.llmProvider);
    this.claimVerifier = new ClaimVerifier(this.config.verification.enabled ? this.llmProvider : undefined);
  }

  /**
   * Quota guard for the Google free tier: at most googleDailyLimit fallback
   * calls per calendar day, counted in the research SQLite store.
   */
  private async claimGoogleQuota(queryCount = 1): Promise<boolean> {
    const limit = this.config.searchFallback?.googleDailyLimit ?? 90;
    const day = new Date().toISOString().split('T')[0];
    const key = `google_search_count_${day}`;
    return this.searchStorage.claimQuota('quota', key, queryCount, limit, 90000);
  }

  /**
   * Health Check: Validates status of dependencies without charging any paid APIs.
   */
  async getHealthStatus(): Promise<{
    search: string;
    database: string;
    reranker: string;
    llm: string;
    google: string;
  }> {
    let searchStatus = this.config.searchProvider === 'google'
      ? this.config.googleApiKey && this.config.googleCxId ? 'configured_not_tested' : 'not_configured' : 'ready';
    try {
      if (this.searchProvider instanceof SearXNGProvider) {
        // HTTP 200 at the root is insufficient: a fixture or broken engine
        // can answer the ping while returning no live search evidence.
        const hits = await this.searchProvider.search({ query: 'search health check', maxResults: 1 });
        searchStatus = hits.length ? 'healthy' : 'no_results';
      }
    } catch (error) {
      searchStatus = /fixture/i.test(String(error)) ? 'fixture' : 'unreachable';
    }

    return {
      search: searchStatus,
      database: this.config.cache.enabled ? 'configured' : 'disabled',
      reranker: this.config.reranking.enabled ? this.reranker.constructor.name : 'disabled',
      llm: this.llmProvider ? 'configured' : 'deterministic_fallback_ready',
      google: this.config.googleApiKey && this.config.googleCxId ? 'configured_not_tested' : 'not_configured',
    };
  }

  /**
   * Fetch + extract one URL with full SSRF/redirect/fetch limits: HTML keeps
   * headings, links, and table structure; PDFs keep page references. Missing
   * text is not labeled OCR-required. Browser rendering fallback is
   * intentionally NOT bundled: JS-dependent pages surface as snippet-only
   * evidence with an explicit limitation (see docs).
   */
  /** Direct search for agent-controlled research; no nested planning/synthesis. */
  async searchQueries(queries: string[], freshness: FreshnessWindow = 'any') {
    if (!Array.isArray(queries) || queries.length < 1 || queries.length > 4 ||
        queries.some(q => typeof q !== 'string' || !q.trim() || new TextEncoder().encode(q).length > 8000)) {
      throw new Error('search requires 1-4 nonempty queries, at most 8000 bytes each');
    }
    const planned = [...new Set(queries.map(q => q.trim()))].map(query => ({ query, purpose: 'agent research', freshness }));
    return this.executeSearches(planned);
  }

  private async executeSearches(queries: PlannedQuery[]) {
    const options = { maxConcurrentQueries: this.config.maxConcurrentQueries, retries: this.config.searchRetries, retryDelayMs: this.config.searchRetryDelayMs };
    const freshness = queries[0]?.freshness ?? 'any';
    const key = JSON.stringify([this.config.resultsPerQuery, queries.map(query => [query.query, query.freshness, query.page ?? 1])]);
    const run = async (provider: SearchProvider) => {
      const name = provider instanceof GoogleSearchProvider ? 'google' : 'searxng';
      const providerKey = JSON.stringify(name === 'google' ? [name, this.config.googleCxId]
        : [name, this.config.searxngBaseUrl, this.config.searxngEngines, this.config.searxngDisabledEngines]);
      const [cached, meta] = this.config.cache.enabled ? await Promise.all([
        this.searchCache.get(providerKey, key, freshness), this.searchCache.getMeta(providerKey, key, freshness),
      ]) : [null, null];
      if (cached?.length) return { results: cached, rawCount: cached.length, failureCount: 0, failures: [], meta: meta ?? emptySearchMeta(), provider: name };
      const result = await new SearchService(provider, options).executeSearches(queries, this.config.resultsPerQuery);
      if (this.config.cache.enabled && result.results.length && result.failureCount === 0) {
        const ttl = ttlForFreshness(freshness, this.config.cache.searchTtlSeconds);
        await this.searchCache.set(providerKey, key, result.results, freshness, ttl);
        await this.searchCache.setMeta(providerKey, key, result.meta, freshness, ttl);
      }
      return { ...result, provider: name };
    };
    const result = await run(this.searchProvider);
    if (result.results.length || !this.fallbackProvider) return { ...result, fallbackUsed: false, fallbackReason: '', primaryFailures: result.failures };
    const fallback = await run(this.fallbackProvider);
    return { ...fallback, provider: fallback.results.length ? fallback.provider : result.provider, fallbackUsed: fallback.results.length > 0,
      fallbackReason: `primary ${result.provider} empty; fallback ${fallback.provider}`, primaryFailures: result.failures };
  }

  async fetchUrl(rawUrl: string, freshness: FreshnessWindow = 'any'): Promise<RetrievedDocument> {
    const ttl = ttlForFreshness(freshness, 86400);
    if (this.config.cache.enabled) {
      const cached = await this.documentCache.get(rawUrl, ttl);
      if (cached) return cached;
    }
    const document = await this.fetchDocument(rawUrl);
    if (this.config.cache.enabled && document.metadata?.extractionMethod !== 'wayback') {
      await this.documentCache.set(document, ttl, rawUrl);
    }
    return document;
  }

  private async fetchDocument(rawUrl: string): Promise<RetrievedDocument> {
    const outcome = await fetchPageCascade(rawUrl, this.fetcher, {
      timeoutSeconds: this.config.fetch.timeoutSeconds,
      maxBytes: this.config.fetch.maxBytes,
      userAgent: this.config.fetch.userAgent,
      waybackFallback: this.config.fetch.waybackFallback,
      domainStats: this.domainStats,
      jsRender: this.jsRender,
    });
    const res = outcome.raw;
    const mime = (res?.mimeType || '').split(';')[0].trim().toLowerCase();
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
        id: `pdf-${Date.now()}`,
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
        id: `fetch-${Date.now()}`,
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
    throw new FetchFailure(
      `Fetch failed: ${outcome.error || res?.error || 'unknown error'}`,
      outcome.raw?.status,
      outcome.raw?.body,
    );
  }

  private async retrieveCandidate(
    candidate: SearchResult,
    index: number,
    freshness: FreshnessWindow,
  ): Promise<{ doc: RetrievedDocument | null; method: string; skippedLive?: boolean }> {
    if (this.config.cache.enabled) {
      const cached = await this.documentCache.get(candidate.url, ttlForFreshness(freshness, 86400));
      if (cached) return { doc: { ...cached, id: `doc-${index + 1}` }, method: 'cache' };
    }

    const outcome = await fetchPageCascade(candidate.url, this.fetcher, {
      timeoutSeconds: this.config.fetch.timeoutSeconds,
      maxBytes: this.config.fetch.maxBytes,
      userAgent: this.config.fetch.userAgent,
      titleHint: candidate.title,
      waybackFallback: this.config.fetch.waybackFallback,
      domainStats: this.domainStats,
      jsRender: this.jsRender,
    });

    if (outcome.document && outcome.document.text.trim().length > 0) {
      const doc: RetrievedDocument = {
        id: `doc-${index + 1}`,
        url: outcome.document.finalUrl || candidate.url,
        canonicalUrl: outcome.document.canonicalUrl || candidate.canonicalUrl,
        domain: candidate.domain,
        title: outcome.document.title || candidate.title,
        author: outcome.document.author || (candidate.metadata?.author as string | undefined),
        publishedAt: outcome.document.publishedAt || candidate.publishedAt,
        text: outcome.document.text,
        headings: outcome.document.headings,
        links: outcome.document.links,
        contentHash: outcome.document.contentHash,
        retrievedAt: new Date().toISOString(),
        searchResultIds: [candidate.id],
        metadata: {
          extractionMethod: outcome.method === 'github_raw' ? 'github_raw'
            : outcome.method === 'js_render' ? 'js_render'
            : outcome.method === 'wayback' ? 'wayback' : 'main_content',
          archivedAt: outcome.document.archivedAt,
        },
      };
      if (this.config.cache.enabled && outcome.method !== 'wayback') {
        await this.documentCache.set(doc, ttlForFreshness(freshness, 86400), candidate.url);
      }
      return { doc, method: outcome.method, skippedLive: outcome.skippedLive };
    }

    if (candidate.snippet && candidate.snippet.trim().length > 0) {
      const fallbackExt = fallbackSnippetExtraction(candidate.title, candidate.snippet, candidate.url);
      return {
        doc: {
          id: `doc-${index + 1}`,
          url: candidate.url,
          canonicalUrl: candidate.canonicalUrl,
          domain: candidate.domain,
          title: fallbackExt.title || candidate.title,
          author: candidate.metadata?.author as string | undefined,
          publishedAt: candidate.publishedAt,
          text: fallbackExt.text,
          contentHash: fallbackExt.contentHash,
          retrievedAt: new Date().toISOString(),
          searchResultIds: [candidate.id],
          metadata: { extractionMethod: 'search_snippet', fetchError: outcome.error || 'fetch failed' },
        },
        method: 'search_snippet',
        skippedLive: outcome.skippedLive,
      };
    }

    return { doc: null, method: 'failed', skippedLive: outcome.skippedLive };
  }

  /**
   * Primary entry point: Executes the entire search, retrieval, grounding, citation,
   * and verification pipeline for a given user question.
   */
  async researchStaged(question: string, options: { mode?: ResearchMode; currentDate?: string; context?: string; objective?: string; fetchPages?: boolean; deadlineMs?: number } = {}): Promise<ResearchSession> {
    if (!this.config.enabled) throw new Error('Web research is disabled');
    if (!question.trim() || question.length > 8000) throw new Error('Question must contain 1–8000 characters');
    const route = routeRequest(normalizeRequest(question).normalizedQuery);
    if (!route.requiresWebSearch || !this.llmProvider) return this.research(question, options);
    const start = Date.now();
    const budget = researchBudget(options.mode ?? this.config.mode);
    const providers = new Set<string>();
    const fallbackReasons = new Set<string>();
    const fetchedDocuments = new Map<string, RetrievedDocument>();
    const result = await runStagedResearch({
      question, context: options.context, objective: options.objective, currentDate: options.currentDate ?? localDateString(),
      freshness: route.freshness, model: this.llmProvider,
      deadlineMs: options.deadlineMs ?? start + budget.timeMs,
      search: async (query, freshness) => {
        const batch = await this.searchQueries([query], freshness);
        providers.add(batch.provider);
        if (batch.fallbackUsed) fallbackReasons.add(batch.fallbackReason);
        return batch;
      },
      fetchPage: options.fetchPages ? async url => {
        const document = await this.fetchUrl(url, route.freshness);
        fetchedDocuments.set(url, document);
        return document;
      } : undefined,
      budget,
      // A comparison still needs at least one query per side and one across
      // them — a floor on the budget, not a clamp on it.
      maxQueries: Math.max(/\b(compare|versus|vs\.?|between)\b/i.test(question) ? 3 : 1,
        budget.initialQueries),
    });
    const documents: RetrievedDocument[] = Object.values(result.sources).map(source => {
      const fetched = fetchedDocuments.get(source.url);
      return fetched ? { ...fetched, id: `doc-${source.id}` } : {
        id: `snippet-${source.id}`, url: source.url, domain: source.domain, title: source.title,
        text: source.snippet ?? '', retrievedAt: new Date().toISOString(), searchResultIds: [],
        metadata: { extractionMethod: source.evidenceKind ?? 'search_snippet' },
      };
    });
    const evidence = result.evidence;
    const evidenceTokens = defaultTokenCounter.count(result.facts.map(fact => fact.text).join('\n'));
    const promptTokens = defaultTokenCounter.count(question) + evidenceTokens;
    return {
      id: `sess-${crypto.randomUUID()}`, question, normalizedQuery: question.trim(), route,
      clarificationQuestion: result.clarificationQuestion,
      queries: result.queries,
      results: result.hits.map((hit, index) => ({ ...hit, id: `hit-${index + 1}`, queryId: '', rank: index + 1 })),
      documents, chunks: [], retrievedChunks: [], rerankedChunks: [], evidence, sources: result.sources,
      verification: result.verification,
      limitations: [...result.limitations, ...(result.facts.length ? ['Facts came from bounded page excerpts or search snippets; full pages were not comprehensively verified.'] : []),
        ...(result.failures.some(failure => /deadline/i.test(failure.error)) ? ['Research deadline reached; returned partial evidence.'] : [])],
      tokenUsage: {
        systemBudget: this.config.context.systemBudget, questionBudget: defaultTokenCounter.count(question),
        evidenceBudget: this.config.context.evidenceTokenBudget, safetyMargin: this.config.context.safetyMargin,
        totalInputBudget: this.config.context.totalInputBudget, estimatedEvidenceTokens: evidenceTokens,
        finalPromptTokens: promptTokens,
      },
      answer: result.answer, startedAt: new Date(start).toISOString(), completedAt: new Date().toISOString(),
      trace: {
        ...new TraceCollector().getTrace(), queriesGenerated: result.queries.length,
        searchResults: result.rawCount, uniqueResults: result.hits.length,
        pagesFetched: result.pagesFetched, fetchFailures: result.fetchFailures,
        searchFailureCount: result.failures.length, queryFailures: result.failures,
        extractionDiagnostics: result.extractionDiagnostics,
        extractedTokens: result.hits.reduce((sum, hit) => sum + defaultTokenCounter.count(hit.snippet), 0),
        evidenceClaims: evidence.length, finalEvidenceTokens: evidenceTokens, finalPromptTokens: promptTokens,
        verifiedClaims: result.verification.supportedCount, unsupportedClaims: result.verification.unsupportedCount,
        conflictingClaims: result.verification.conflictingCount,
        providerUsed: [...providers].join(',') || this.config.searchProvider, fallbackUsed: fallbackReasons.size > 0,
        fallbackReason: [...fallbackReasons].join('; '), totalLatencyMs: Date.now() - start,
      },
    };
  }

  async research(
    question: string,
    options: { mode?: ResearchMode; currentDate?: string } = {}
  ): Promise<ResearchSession> {
    if (!this.config.enabled) throw new Error('Web research is disabled');
    if (!question.trim() || question.length > 8000) throw new Error('Question must contain 1–8000 characters');
    const startTime = Date.now();
    const mode = options.mode || this.config.mode;
    const curDate = options.currentDate || new Date().toISOString().split('T')[0];

    const trace = new TraceCollector();
    trace.startTimer('total');

    // Step 0: REQUEST NORMALIZATION (Section 8)
    trace.startTimer('normalization');
    const normalizedReq = normalizeRequest(question);
    trace.endTimer('normalization');

    // Step 1: ROUTE
    trace.startTimer('route');
    const route = routeRequest(normalizedReq.normalizedQuery);
    trace.endTimer('route');

    const sourceManager = new SourceManager();

    // Fast path A: Static Knowledge (No external search required)
    if (!route.requiresExternalData) {
      const defaultStats: TokenStats = {
        systemBudget: this.config.context.systemBudget,
        questionBudget: defaultTokenCounter.count(question),
        evidenceBudget: 0,
        safetyMargin: this.config.context.safetyMargin,
        totalInputBudget: this.config.context.totalInputBudget,
        estimatedEvidenceTokens: 0,
        finalPromptTokens: defaultTokenCounter.count(question),
      };

      const ans = await this.answerGenerator.generateAnswer(
        {
          systemPrompt: 'You are a helpful AI assistant. Answer the user question directly and accurately.',
          userPrompt: question,
          evidenceTokens: 0,
        },
        {},
        [],
        question
      );

      trace.endTimer('total');
      const finalTrace = trace.getTrace();
      finalTrace.totalLatencyMs = Date.now() - startTime;

      return {
        id: `sess-${crypto.randomUUID()}`,
        question,
        normalizedQuery: normalizedReq.normalizedQuery,
        route,
        queries: [],
        results: [],
        documents: [],
        chunks: [],
        retrievedChunks: [],
        rerankedChunks: [],
        evidence: [],
        sources: {},
        tokenUsage: defaultStats,
        answer: ans.markdownWithCitations,
        startedAt: new Date(startTime).toISOString(),
        completedAt: new Date().toISOString(),
        trace: finalTrace,
      };
    }

    // Fast path B: Structured Verticals (Weather, Currency, Time)
    // A provider that throws or returns nothing falls through to the standard
    // web-search path; the canned error is kept only as a last resort below.
    let verticalError = '';
    if (!route.requiresWebSearch) {
      let verticalData = '';
      let sourceUrl = '';
      try {
      if (route.vertical === 'WEATHER') {
        const report = await this.weatherProvider.execute(question);
        sourceUrl = report.sourceUrl;
        verticalData = this.weatherProvider.formatReportAsEvidence(report);
      } else if (route.vertical === 'CURRENCY') {
        const report = await this.currencyProvider.execute(question);
        sourceUrl = report.sourceUrl;
        verticalData = this.currencyProvider.formatReportAsEvidence(report);
      } else if (route.vertical === 'TIME') {
        const report = await this.timeProvider.execute(question);
        verticalData = this.timeProvider.formatReportAsEvidence(report);
      }
      } catch (error) {
        verticalError = `I couldn't retrieve current ${route.vertical.toLowerCase()} data. ${error instanceof Error ? error.message : 'Provider unavailable.'}`;
      }

      if (!verticalError && verticalData) {
      // Register vertical pseudo-source
      const sourceId = sourceUrl ? sourceManager.registerDocument({
        id: 'VERT-1',
        url: sourceUrl,
        domain: new URL(sourceUrl).hostname,
        title: `${route.vertical} Service`,
        text: verticalData,
        retrievedAt: new Date().toISOString(),
        searchResultIds: [],
      }) : '';

      const verticalClaim: EvidenceClaim = {
        id: 'CLM-V1',
        claim: verticalData,
        supportingSources: sourceId ? [sourceId] : [],
        status: 'supported',
        confidence: 0.99,
      };

      const context = buildGroundedContext(question, sourceManager.getAllSources(), [verticalClaim], {
        currentDate: curDate,
      });

      // Structured measurements already answer the question. Do not let generation
      // introduce a different number, location, rate, or timestamp.
      const ans = this.answerGenerator.deterministicSynthesize(question, [verticalClaim], sourceManager.getAllSources());
      if (route.vertical === 'TIME') ans.markdownWithCitations = verticalData + '\nSource: local system clock.';

      trace.endTimer('total');
      const finalTrace = trace.getTrace();
      finalTrace.totalLatencyMs = Date.now() - startTime;
      finalTrace.evidenceClaims = 1;
      finalTrace.verifiedClaims = 0;
      finalTrace.finalEvidenceTokens = context.evidenceTokens;

      const tokenStats: TokenStats = {
        systemBudget: this.config.context.systemBudget,
        questionBudget: defaultTokenCounter.count(question),
        evidenceBudget: this.config.context.evidenceTokenBudget,
        safetyMargin: this.config.context.safetyMargin,
        totalInputBudget: this.config.context.totalInputBudget,
        estimatedEvidenceTokens: context.evidenceTokens,
        finalPromptTokens: context.evidenceTokens + defaultTokenCounter.count(question),
      };

      return {
        id: `sess-${crypto.randomUUID()}`,
        question,
        normalizedQuery: normalizedReq.normalizedQuery,
        route,
        queries: [],
        results: [],
        documents: [],
        chunks: [],
        retrievedChunks: [],
        rerankedChunks: [],
        evidence: [verticalClaim],
        sources: sourceManager.getAllSources(),
        tokenUsage: tokenStats,
        answer: ans.markdownWithCitations,
        startedAt: new Date(startTime).toISOString(),
        completedAt: new Date().toISOString(),
        trace: finalTrace,
      };
      }
    }

    // Standard Path: Full Web Search Pipeline
    // Step 2: Query Planning
    trace.startTimer('planning');
    const budget = researchBudget(mode);
    let maxQueries = budget.initialQueries;
    const totalQueryBudget = maxQueries;
    const retryReserve = this.config.verification.enabled ? Math.min(budget.retries, Math.max(0, maxQueries - 1)) : 0;
    maxQueries -= retryReserve;

    const plannedQueries = await this.queryPlanner.planQueries(
      question,
      route.freshness,
      maxQueries,
      curDate
    );
    trace.endTimer('planning');
    trace.update({ queriesGenerated: plannedQueries.length });

    // Step 3: Concurrent Search Execution with Caching
    trace.startTimer('search');
    const batch = await this.executeSearches(plannedQueries);
    let searchOutcome: { results: SearchResult[]; rawCount: number; failureCount: number; failures?: Array<{ query: string; error: string }>; meta: SearchMeta } = batch;
    const queryFailures = batch.fallbackUsed ? [...batch.primaryFailures, ...batch.failures] : batch.primaryFailures;
    trace.update({
      queryFailures, searchFailureCount: queryFailures.length,
      providerUsed: batch.provider, fallbackUsed: batch.fallbackUsed, fallbackReason: batch.fallbackReason,
    });
    if (this.searchProvider instanceof SearXNGProvider && this.searchProvider.lastDiagnostics) {
      trace.update({ searxngDiagnostics: { ...this.searchProvider.lastDiagnostics } });
    }
    trace.endTimer('search');
    trace.update({
      searchResults: searchOutcome.rawCount,
      uniqueResults: searchOutcome.results.length,
    });

    // Selective pagination: when page one yields nothing usable but queries
    // succeeded, spend one budgeted page-two fetch per query (honors the
    // configured query budget) before declaring the pool empty.
    if (this.config.enablePagination && searchOutcome.results.length === 0 && searchOutcome.failureCount === 0) {
      const paged = new SearchService(this.searchProvider, {
        maxConcurrentQueries: this.config.maxConcurrentQueries,
        retries: 0,
      });
      const pagedResults: typeof searchOutcome.results = [];
      const pagedPages: number[] = [];
      let pagedRaw = 0;
      const remaining = Math.max(0, totalQueryBudget - plannedQueries.length);
      for (const q of plannedQueries.slice(0, Math.max(1, remaining))) {
        const extra = await paged.fetchAdditionalPage(q, 2, this.config.resultsPerQuery);
        if (extra.results.length > 0) {
          pagedResults.push(...extra.results);
          pagedRaw += extra.results.length;
          pagedPages.push(2);
          plannedQueries.push({ ...q, page: 2, purpose: `${q.purpose} (page 2)` });
          if (extra.meta) searchOutcome.meta = mergeSearchMeta(searchOutcome.meta, extra.meta);
        }
      }
      if (pagedResults.length > 0) {
        const fused = fuseSearchResults([{ query: 'page-2', results: pagedResults }]);
        searchOutcome = { ...searchOutcome, results: fused, rawCount: searchOutcome.rawCount + pagedRaw };
        trace.update({
          searchResults: searchOutcome.rawCount,
          uniqueResults: fused.length,
          paginationPages: pagedPages,
        });
      }
    }

    // SearXNG related-search suggestions: one extra query when the first pass
    // is thin, without another LLM round-trip. Cheap recall for small models.
    if (
      searchOutcome.meta.suggestions.length > 0 &&
      searchOutcome.results.length < 3 &&
      plannedQueries.length < totalQueryBudget
    ) {
      const suggestion = searchOutcome.meta.suggestions.find(
        (s) => s.trim().length > 0 && !plannedQueries.some((q) => q.query.toLowerCase() === s.toLowerCase()),
      );
      if (suggestion) {
        const extra = await new SearchService(this.searchProvider, { retries: 0 }).executeSearches(
          [{ query: suggestion, purpose: 'SearXNG related-search suggestion', freshness: route.freshness }],
          this.config.resultsPerQuery,
        );
        if (extra.results.length > 0) {
          plannedQueries.push({ query: suggestion, purpose: 'SearXNG related-search suggestion', freshness: route.freshness });
          const fused = fuseSearchResults([
            { query: 'primary', results: searchOutcome.results },
            { query: suggestion, results: extra.results },
          ]);
          searchOutcome = {
            ...searchOutcome,
            results: fused,
            rawCount: searchOutcome.rawCount + extra.rawCount,
            meta: mergeSearchMeta(searchOutcome.meta, extra.meta),
          };
        }
      }
    }

    // Step 4: Multi-Factor Scoring & Ranking
    trace.startTimer('ranking');
    const rankedResults = rankSearchResults(searchOutcome.results.filter(r => isSafeUrl(r.url)), {
      query: question,
      freshness: route.freshness,
      vertical: route.vertical,
      totalPlannedQueries: plannedQueries.length,
      weights: this.config.ranking,
    });
    trace.endTimer('ranking');

    // Step 5: Page Fetch Selection
    const pageBudget = budget.pageFetches;
    let fetchPool = selectPagesToFetch(rankedResults, Math.max(1, pageBudget - retryReserve * 2));
    let domainsDeprioritized: string[] = [];
    if (this.domainStats) {
      const ordered = await orderByDomainHealth(fetchPool, this.domainStats);
      fetchPool = ordered.ordered;
      domainsDeprioritized = ordered.deprioritized;
    }
    const pagesToFetch = fetchPool;
    let pagesAttempted = pagesToFetch.length;
    trace.update({ pagesSelected: pagesToFetch.length, domainsDeprioritized });


    // Instant answers / Wikipedia infoboxes are already the answer for many
    // factoid questions. Inject them before page fetches so small models see
    // structured evidence even when HTML extraction fails.
    const documents: RetrievedDocument[] = documentsFromSearchMeta(searchOutcome.meta);

    // Step 6: Safe Page Fetching & Main Content Extraction
    trace.startTimer('fetch_extract');
    let fetchFailures = 0;
    let successfulFetches = 0;
    let extractedTokens = documents.reduce((sum, d) => sum + defaultTokenCounter.count(d.text), 0);
    let waybackRecoveries = 0;
    const domainsChronicSkipped: string[] = [];

    const fetchPromises = pagesToFetch.map(async (candidate, index) => {
      return this.retrieveCandidate(candidate, index + documents.length, route.freshness);
    });

    const settledDocs = await Promise.allSettled(fetchPromises);
    for (const res of settledDocs) {
      if (res.status === 'fulfilled' && res.value.doc) {
        const { doc, method, skippedLive } = res.value;
        if (method === 'wayback') waybackRecoveries++;
        if (method === 'live' || method === 'github_raw' || method === 'js_render' || method === 'wayback' || method === 'cache') successfulFetches++;
        else fetchFailures++;
        if (skippedLive && !domainsChronicSkipped.includes(doc.domain)) domainsChronicSkipped.push(doc.domain);
        extractedTokens += defaultTokenCounter.count(doc.text);
        if (!documents.some(d => d.contentHash && d.contentHash === doc.contentHash)) documents.push(doc);
      } else if (res.status === 'rejected') {
        fetchFailures++;
      } else {
        fetchFailures++;
      }
    }
    trace.endTimer('fetch_extract');
    trace.update({
      pagesFetched: successfulFetches,
      fetchFailures,
      extractedTokens,
      waybackRecoveries,
      domainsChronicSkipped,
    });


    // Snippet salvage: when nothing was extracted but ranked candidates carry
    // snippets, build snippet-only docs rather than returning empty evidence.
    const salvageLimitations: string[] = [];
    if (documents.length === 0) {
      const salvaged = rankedResults
        .filter(r => isSafeUrl(r.url) && r.snippet && r.snippet.trim().length > 0)
        .slice(0, Math.max(1, pageBudget - fetchFailures));
      for (const [i, candidate] of salvaged.entries()) {
        const fallbackExt = fallbackSnippetExtraction(candidate.title, candidate.snippet!, candidate.url);
        documents.push({
          id: `doc-salvage-${i + 1}`,
          url: candidate.url,
          canonicalUrl: candidate.canonicalUrl,
          domain: candidate.domain,
          title: fallbackExt.title || candidate.title,
          text: fallbackExt.text,
          contentHash: fallbackExt.contentHash,
          retrievedAt: new Date().toISOString(),
          searchResultIds: [candidate.id],
          metadata: { extractionMethod: 'snippet_salvage', fetchError: 'all fetches failed; snippet-only evidence, low confidence' },
        });
      }
      if (documents.length > 0) {
        salvageLimitations.push(`${fetchFailures} pages blocked or failed; showing snippet-only evidence, low confidence.`);
      }
    }

    // Step 7: Semantic Document Chunking
    trace.startTimer('chunking');
    const allChunks: EvidenceChunk[] = [];
    for (const doc of documents) {
      // Register document in source registry
      sourceManager.registerDocument(doc);

      const chunks = chunkDocument(doc, {
        targetTokens: this.config.chunking.targetTokens,
        overlapTokens: this.config.chunking.overlapTokens,
      });
      allChunks.push(...chunks);
    }
    trace.endTimer('chunking');
    trace.update({ chunksCreated: allChunks.length });

    // Step 8: Retrieval (BM25 + per-document diversity cap)
    trace.startTimer('retrieval');
    const candidateChunks = retrieveChunks(question, allChunks, {
      candidateLimit: this.config.retrieval.candidateLimit,
      maxChunksPerDoc: this.config.retrieval.maxChunksPerDoc,
    });
    trace.endTimer('retrieval');
    trace.update({ chunksAfterRetrieval: candidateChunks.length });

    // Step 9: Cross-Encoder Reranking
    trace.startTimer('rerank');
    const rerankedChunks = await this.reranker.rerank(
      question,
      candidateChunks,
      this.config.retrieval.finalLimit
    );
    trace.endTimer('rerank');
    trace.update({ chunksAfterRerank: rerankedChunks.length });

    // Step 10: Evidence Extraction & Compression
    trace.startTimer('evidence');
    const extractedFacts: ExtractedFact[] = [];
    for (const chunk of rerankedChunks) {
      const sourceId = sourceManager.getSourceIdForUrl(chunk.url) || 'S1';
      const facts = await this.evidenceExtractor.extractFacts(question, chunk, sourceId);
      extractedFacts.push(...facts);
    }

    // Step 11: Deduplication & Conflict Detection
    const deduplicatedClaims = deduplicateFacts(extractedFacts);
    const finalClaims = detectEvidenceConflicts(deduplicatedClaims);
    trace.endTimer('evidence');
    trace.update({ evidenceClaims: finalClaims.length });

    // Step 12: Context Building & Token Budget Management
    trace.startTimer('context_builder');
    let {allocation: budgetAllocation, context: groundedContext} = await buildModelBudgetedContext(question, sourceManager.getAllSources(), finalClaims, this.config.context, curDate, this.llmProvider?.countTokens?.bind(this.llmProvider));
    trace.endTimer('context_builder');
    trace.update({
      finalEvidenceTokens: groundedContext.evidenceTokens,
      finalPromptTokens: budgetAllocation.tokenStats.finalPromptTokens,
    });

    // Step 13: Grounded Answer Synthesis
    trace.startTimer('generation');
    let answerOutcome = await this.answerGenerator.generateAnswer(
      groundedContext,
      sourceManager.getAllSources(),
      budgetAllocation.claims,
      question
    );
    trace.endTimer('generation');

    // Step 14: Claim Verification
    trace.startTimer('verification');
    const atomicClaims = extractAtomicClaims(answerOutcome.text);
    let verificationReport = await this.claimVerifier.verifyClaims(
      atomicClaims,
      budgetAllocation.claims
    );
    let coverage = coverageMatrix(question, budgetAllocation.claims);
    trace.endTimer('verification');
    trace.update({
      verifiedClaims: verificationReport.supportedCount,
      unsupportedClaims: verificationReport.unsupportedCount,
      conflictingClaims: verificationReport.conflictingCount,
    });

    // Step 15: Iterative Research Retry Loop (Section 56 & Section 127)
    // Structured research state tracks requirements, findings, contradictions,
    // and stagnation; repeated actions and evidence-free rounds end the loop.
    const researchState = initResearchState(question);
    for (const claim of budgetAllocation.claims) {
      recordRound(researchState, `initial: ${plannedQueries.map((q) => q.query).join('; ')}`, [claim], sourceManager.getAllSources());
    }
    researchState.completedActions = [`initial: ${plannedQueries.map((q) => q.query).join('; ')}`];
    let retryCount = 0;
    while (
      this.config.verification.enabled &&
      (!verificationReport.allSupported || coverage.some(cell => !cell.covered)) &&
      retryCount < budget.retries && plannedQueries.length < totalQueryBudget && pagesAttempted < pageBudget
    ) {
      const retryEval = evaluateResearchRetry(
        verificationReport,
        retryCount,
        budget.retries,
        plannedQueries.map((q) => q.query),
        researchState.roundsWithoutNewEvidence
      );
      const missing = coverage.find(cell => !cell.covered);
      if (missing) {
        const gapQuery = `${missing.entity} ${missing.dimension} official`;
        if (!isRepeatedQuery(gapQuery, plannedQueries.map((q) => q.query))) {
          retryEval.shouldRetry = true;
          retryEval.targetedQuery = {query:gapQuery,purpose:'Fill missing comparison evidence',freshness:route.freshness};
        }
      }

      if (!retryEval.shouldRetry || !retryEval.targetedQuery) {
        break;
      }

      retryCount++;
      trace.startTimer(`retry_${retryCount}`);

      // 1. Search targeted query
      const searchService = new SearchService(this.searchProvider);
      const retrySearchOutcome = await searchService.executeSearches(
        [retryEval.targetedQuery],
        this.config.resultsPerQuery
      );

      if (retrySearchOutcome.results.length > 0) {
        plannedQueries.push(retryEval.targetedQuery);
        const newRanked = rankSearchResults(retrySearchOutcome.results.filter(r => isSafeUrl(r.url)), {
          query: retryEval.targetedQuery.query,
          freshness: retryEval.targetedQuery.freshness,
          vertical: route.vertical,
          totalPlannedQueries: plannedQueries.length,
          weights: this.config.ranking,
        });

        const retryPages = selectPagesToFetch(newRanked, Math.min(Math.max(1, Math.floor(pageBudget / 4)), pageBudget - pagesAttempted)).filter(r => !documents.some(d => d.url === r.url));
        pagesAttempted += retryPages.length;
        rankedResults.push(...newRanked.filter(r => !rankedResults.some(old => old.url === r.url)));
        const retryChunks: EvidenceChunk[] = [];

        for (const candidate of retryPages) {
          const retrieved = await this.retrieveCandidate(candidate, documents.length, route.freshness);
          const retryDoc = retrieved.doc;
          if (retrieved.method === 'live' || retrieved.method === 'github_raw' || retrieved.method === 'js_render' || retrieved.method === 'wayback' || retrieved.method === 'cache') {
            successfulFetches++;
            if (retrieved.method === 'wayback') waybackRecoveries++;
          } else {
            fetchFailures++;
          }

          if (retryDoc) {
            extractedTokens += defaultTokenCounter.count(retryDoc.text);
            documents.push(retryDoc);
            sourceManager.registerDocument(retryDoc);
            const chunks = chunkDocument(retryDoc, {
              targetTokens: this.config.chunking.targetTokens,
              overlapTokens: this.config.chunking.overlapTokens,
            });
            allChunks.push(...chunks);
            retryChunks.push(...chunks);
          }
        }

        if (retryChunks.length > 0) {
          const retryCandidateChunks = retrieveChunks(
            retryEval.targetedQuery.query,
            retryChunks,
            { candidateLimit: 5, maxChunksPerDoc: this.config.retrieval.maxChunksPerDoc }
          );
          const retryReranked = await this.reranker.rerank(
            retryEval.targetedQuery.query,
            retryCandidateChunks,
            3
          );

          for (const chunk of retryReranked) {
            const sId = sourceManager.getSourceIdForUrl(chunk.url) || 'S1';
            const facts = await this.evidenceExtractor.extractFacts(
              retryEval.targetedQuery.query,
              chunk,
              sId
            );
            extractedFacts.push(...facts);
          }

          const updatedDeduplicated = deduplicateFacts(extractedFacts);
          const updatedFinalClaims = detectEvidenceConflicts(updatedDeduplicated);
          recordRound(researchState, retryEval.targetedQuery.query, updatedFinalClaims, sourceManager.getAllSources());
          const {allocation: updatedBudget, context: updatedContext} = await buildModelBudgetedContext(question, sourceManager.getAllSources(), updatedFinalClaims, this.config.context, curDate, this.llmProvider?.countTokens?.bind(this.llmProvider));
          budgetAllocation = updatedBudget;
          groundedContext = updatedContext;
          coverage = coverageMatrix(question, updatedBudget.claims);

          answerOutcome = await this.answerGenerator.generateAnswer(
            updatedContext,
            sourceManager.getAllSources(),
            updatedBudget.claims,
            question
          );

          const newAtomicClaims = extractAtomicClaims(answerOutcome.text);
          verificationReport = await this.claimVerifier.verifyClaims(
            newAtomicClaims,
            updatedBudget.claims
          );

          trace.update({
            finalEvidenceTokens: updatedContext.evidenceTokens,
            finalPromptTokens: updatedBudget.tokenStats.finalPromptTokens,
            evidenceClaims: updatedBudget.claims.length,
            verifiedClaims: verificationReport.supportedCount,
            unsupportedClaims: verificationReport.unsupportedCount,
            conflictingClaims: verificationReport.conflictingCount,
          });
        }
      }
      trace.endTimer(`retry_${retryCount}`);
    }

    // Never publish an unsupported draft just because retry budget ran out.
    if (this.config.verification.enabled && (!verificationReport.allSupported || answerOutcome.invalidSourceIds.length > 0)) {
      answerOutcome = this.answerGenerator.deterministicSynthesize(question, budgetAllocation.claims, sourceManager.getAllSources());
      answerOutcome.markdownWithCitations += '\n\nSome requested details could not be independently verified; the statements above are attributed to the retrieved sources.';
      verificationReport = this.claimVerifier.verifyClaimsDeterministic(extractAtomicClaims(answerOutcome.text), budgetAllocation.claims);
    }
    trace.update({queriesGenerated: plannedQueries.length, chunksCreated: allChunks.length,
      pagesSelected: pagesAttempted, pagesFetched: successfulFetches, fetchFailures, extractedTokens,
      uniqueResults: rankedResults.length,
      verifiedClaims: verificationReport.supportedCount, unsupportedClaims: verificationReport.unsupportedCount,
      conflictingClaims: verificationReport.conflictingCount});
    trace.endTimer('total');
    const finalTrace = trace.getTrace();
    finalTrace.totalLatencyMs = Date.now() - startTime;

    if (coverage.some(c => !c.covered)) answerOutcome.markdownWithCitations += '\n\nMissing comparison evidence: ' + coverage.filter(c => !c.covered).map(c => `${c.entity} (${c.dimension})`).join(', ') + '.';

    // Requirement classification: answered / unresolved / blocked with a
    // reason. A fluent final response is never proof of completion.
    const requirements = classifyRequirements(researchState, budgetAllocation.claims);
    const unresolved = requirements.filter((r) => r.status !== 'answered');
    if (unresolved.length > 0) {
      answerOutcome.markdownWithCitations += '\n\nUnresolved: ' + unresolved.map((r) => `${r.text} (${r.status}${r.reason ? `: ${r.reason}` : ''})`).join('; ') + '.';
    }

    // Last resort for a structured vertical that skipped search and failed:
    // keep the canned error only when the web-search pass found no evidence.
    if (verticalError && budgetAllocation.claims.length === 0) {
      answerOutcome.markdownWithCitations = verticalError;
    }

    const session: ResearchSession = {
      coverage,
      requirements,
      researchState: {
        openQuestions: researchState.openQuestions,
        contradictions: researchState.contradictions,
        rejected: researchState.rejected,
        completedActions: researchState.completedActions,
        pendingActions: researchState.pendingActions,
      },
      limitations: [
        ...salvageLimitations,
        ...coverage.filter(c => !c.covered).map(c => `Could not verify ${c.entity}: ${c.dimension}`),
        ...unresolved.map((r) => `${r.id} ${r.status}: ${r.text}${r.reason ? ` — ${r.reason}` : ''}`),
      ],
      id: `sess-${crypto.randomUUID()}`,
      question,
      normalizedQuery: normalizedReq.normalizedQuery,
      route,
      queries: plannedQueries,
      results: rankedResults,
      searchMeta: searchOutcome.meta,
      documents,
      chunks: allChunks,
      retrievedChunks: candidateChunks,
      rerankedChunks,
      evidence: budgetAllocation.claims,
      sources: sourceManager.getAllSources(),
      tokenUsage: budgetAllocation.tokenStats,
      answer: answerOutcome.markdownWithCitations,
      verification: verificationReport,
      startedAt: new Date(startTime).toISOString(),
      completedAt: new Date().toISOString(),
      trace: finalTrace,
    };

    return session;
  }
}

