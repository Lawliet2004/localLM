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
  EmbeddingProvider,
  Reranker,
  ResearchSession,
  ResearchMode,
  RetrievedDocument,
  EvidenceChunk,
  ExtractedFact,
  EvidenceClaim,
  TokenStats,
} from './types';
import { createConfig } from './config/defaults';
import type { WebSearchConfig } from './config/schema';
import { fuseSearchResults } from './search/result_fusion';
import { extractPdfText } from './documents/document_store';
import { normalizeRequest } from './router/request_normalizer';
import { routeRequest } from './router/intent_router';
import { QueryPlanner } from './planning/query_planner';
import { SearchService } from './search/search_service';
import { SearXNGProvider } from './search/searxng_provider';
import { GoogleSearchProvider } from './search/google_provider';
import { rankSearchResults } from './ranking/search_ranker';
import { selectPagesToFetch } from './fetch/fetch_policy';
import { HttpFetcher } from './fetch/http_fetcher';
import { extractMainContent } from './extraction/main_content';
import { fallbackSnippetExtraction } from './extraction/extraction_fallback';
import { chunkDocument } from './chunking/semantic_chunker';
import { LocalHashingEmbeddingProvider } from './retrieval/embeddings';
import { HybridRetriever } from './retrieval/hybrid_retriever';
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
import { PipelineLogger } from './observability/logger';
import { TraceCollector } from './observability/trace';
import { defaultTokenCounter } from './chunking/tokenizer';
import type { StorageAdapter } from './cache/sqlite';
import { LocalEmbeddingProvider, LocalModelReranker } from './models/retrieval_providers';
import { coverageMatrix } from './planning/coverage';
import { isSafeUrl } from './security/ssrf_guard';

export * from './types';
export * from './config/schema';
export * from './config/defaults';
export { formatDiagnosticReport } from './observability/metrics';

export interface EngineDependencies {
  searchProvider?: SearchProvider;
  llmProvider?: LLMProvider;
  embeddingProvider?: EmbeddingProvider;
  reranker?: Reranker;
  fetcher?: Pick<HttpFetcher, 'fetch'>;
  storage?: StorageAdapter;
  config?: Partial<WebSearchConfig>;
}

export class WebSearchEngine {
  public config: WebSearchConfig;
  private searchProvider: SearchProvider;
  private fallbackProvider?: SearchProvider;
  private searchStorage?: StorageAdapter;
  private llmProvider?: LLMProvider;
  private embeddingProvider: EmbeddingProvider;
  private reranker: Reranker;
  private fetcher: Pick<HttpFetcher, 'fetch'>;
  private searchCache: SearchCache;
  private documentCache: DocumentCache;
  private weatherProvider: WeatherProvider;
  private currencyProvider: CurrencyProvider;
  private timeProvider: TimeProvider;
  private queryPlanner: QueryPlanner;
  private evidenceExtractor: EvidenceExtractor;
  private answerGenerator: AnswerGenerator;
  private claimVerifier: ClaimVerifier;
  private logger: PipelineLogger;

  constructor(deps: EngineDependencies = {}) {
    this.config = createConfig(deps.config);
    if (this.config.searchProvider === 'google' && this.config.googleApiKey && this.config.googleCxId) {
      this.searchProvider = deps.searchProvider || new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId);
      this.fallbackProvider = (!deps.searchProvider && this.config.searchFallback?.enabled)
        ? new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines) : undefined;
    } else {
      this.searchProvider = deps.searchProvider || new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines);
      this.fallbackProvider = (!deps.searchProvider && this.config.searchFallback?.enabled && this.config.googleApiKey && this.config.googleCxId)
        ? new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId) : undefined;
    }
    this.llmProvider = deps.llmProvider;
    this.embeddingProvider = deps.embeddingProvider || (this.config.localEmbedding ? new LocalEmbeddingProvider(this.config.localEmbedding, deps.storage) : new LocalHashingEmbeddingProvider());
    this.reranker = deps.reranker || (this.config.localReranker && this.config.reranking.enabled ? new LocalModelReranker(this.config.localReranker) : new RerankerService(this.config.reranking.enabled));
    this.fetcher =
      deps.fetcher ||
      new HttpFetcher(
        this.config.fetch.globalConcurrency,
        this.config.fetch.perDomainConcurrency
      );
    this.searchCache = new SearchCache(deps.storage);
    this.documentCache = new DocumentCache(deps.storage);
    this.searchStorage = deps.storage;
    this.weatherProvider = new WeatherProvider();
    this.currencyProvider = new CurrencyProvider();
    this.timeProvider = new TimeProvider();
    this.queryPlanner = new QueryPlanner(this.llmProvider);
    this.evidenceExtractor = new EvidenceExtractor(this.config.extractEvidenceWithModel ? this.llmProvider : undefined);
    this.answerGenerator = new AnswerGenerator(this.llmProvider);
    this.claimVerifier = new ClaimVerifier(this.config.verification.enabled ? this.llmProvider : undefined);
    this.logger = new PipelineLogger(false);
  }

  /**
   * Quota guard for the Google free tier: at most googleDailyLimit fallback
   * calls per calendar day, counted in the research SQLite store.
   */
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

  /**
   * Health Check: Validates status of dependencies without charging any paid APIs.
   */
  async getHealthStatus(): Promise<{
    search: string;
    database: string;
    embeddingModel: string;
    reranker: string;
    llm: string;
    google: string;
  }> {
    let searchStatus = 'ready';
    try {
      if (this.searchProvider instanceof SearXNGProvider) {
        // Ping health
        const res = await fetch(new URL('/', this.config.searxngBaseUrl).toString(), {
          method: 'HEAD',
          signal: AbortSignal.timeout(5000),
          redirect: 'error',
        });
        searchStatus = res.ok ? 'healthy' : `http_${res.status}`;
      }
    } catch {
      searchStatus = 'unreachable';
    }

    return {
      search: searchStatus,
      database: this.config.cache.enabled ? 'configured' : 'disabled',
      embeddingModel: this.config.retrieval.embeddings ? this.embeddingProvider.constructor.name : 'disabled',
      reranker: this.config.reranking.enabled ? this.reranker.constructor.name : 'disabled',
      llm: this.llmProvider ? 'configured' : 'deterministic_fallback_ready',
      google: this.config.googleApiKey && this.config.googleCxId ? 'configured' : 'not_configured',
    };
  }

  /**
   * Fetch + extract one URL with full SSRF/redirect/fetch limits: HTML keeps
   * headings, links, and table structure; PDFs keep page references; scanned
   * PDFs report ocrRequired explicitly. Browser rendering fallback is
   * intentionally NOT bundled: JS-dependent pages surface as snippet-only
   * evidence with an explicit limitation (see docs).
   */
  async fetchUrl(rawUrl: string): Promise<RetrievedDocument> {
    const res = await this.fetcher.fetch(rawUrl, {
      timeoutSeconds: this.config.fetch.timeoutSeconds,
      maxBytes: this.config.fetch.maxBytes,
      userAgent: this.config.fetch.userAgent,
    });
    if (!res.success || !res.body) {
      throw new Error(`Fetch failed: ${res.error || 'unknown error'}`);
    }
    const mime = (res.mimeType || '').split(';')[0].trim().toLowerCase();
    if (mime === 'application/pdf' || /\.pdf(\?|#|$)/i.test(res.finalUrl || rawUrl)) {
      const latin1 = res.body || '';
      const bytes = Uint8Array.from(latin1, (ch) => ch.charCodeAt(0) & 0xff);
      const pdf = extractPdfText(bytes, rawUrl);
      if (pdf.needsOcr) {
        throw new Error(
          'OCR required: this PDF has no extractable text (scanned document). ' +
          'No OCR engine is bundled; install an OCR tool or supply the text directly.',
        );
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
    const extraction = extractMainContent(res.body, rawUrl);
    return {
      id: `fetch-${Date.now()}`,
      url: res.finalUrl || rawUrl,
      domain: new URL(res.finalUrl || rawUrl).hostname,
      title: extraction.title || rawUrl,
      text: extraction.text,
      headings: extraction.headings,
      links: extraction.links,
      retrievedAt: new Date().toISOString(),
      searchResultIds: [],
      metadata: { extractionMethod: 'main_content' },
    };
  }

  /**
   * Primary entry point: Executes the entire search, retrieval, grounding, citation,
   * and verification pipeline for a given user question.
   */
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

    this.logger.log('request_received', { question, mode });

    // Step 0: REQUEST NORMALIZATION (Section 8)
    trace.startTimer('normalization');
    const normalizedReq = normalizeRequest(question);
    trace.endTimer('normalization');
    this.logger.log('request_normalized', { normalizedReq });

    // Step 1: ROUTE
    trace.startTimer('route');
    const route = routeRequest(normalizedReq.normalizedQuery);
    trace.endTimer('route');
    this.logger.log('route_selected', { route });

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
    if (!route.requiresWebSearch) {
      let verticalData = '';
      let sourceUrl = '';
      let verticalError = '';
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
      const ans = this.answerGenerator.deterministicSynthesize(question, verticalError ? [] : [verticalClaim], sourceManager.getAllSources());
      if (verticalError) ans.markdownWithCitations = verticalError;
      if (route.vertical === 'TIME' && !verticalError) ans.markdownWithCitations = verticalData + '\nSource: local system clock.';

      trace.endTimer('total');
      const finalTrace = trace.getTrace();
      finalTrace.totalLatencyMs = Date.now() - startTime;
      finalTrace.evidenceClaims = verticalError ? 0 : 1;
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
        evidence: verticalError ? [] : [verticalClaim],
        sources: sourceManager.getAllSources(),
        tokenUsage: tokenStats,
        answer: ans.markdownWithCitations,
        startedAt: new Date(startTime).toISOString(),
        completedAt: new Date().toISOString(),
        trace: finalTrace,
      };
    }

    // Standard Path: Full Web Search Pipeline
    // Step 2: Query Planning
    trace.startTimer('planning');
    let maxQueries = this.config.queries.normal;
    if (mode === 'fast') maxQueries = this.config.queries.fast;
    if (mode === 'deep') maxQueries = this.config.queries.deep;
    const totalQueryBudget = maxQueries;
    const retryReserve = this.config.verification.enabled ? Math.min(this.config.verification.maxResearchRetries, Math.max(0, maxQueries - 1)) : 0;
    maxQueries -= retryReserve;

    const plannedQueries = await this.queryPlanner.planQueries(
      question,
      route.freshness,
      maxQueries,
      curDate
    );
    trace.endTimer('planning');
    trace.update({ queriesGenerated: plannedQueries.length });
    this.logger.log('queries_generated', { plannedQueries });

    // Step 3: Concurrent Search Execution with Caching
    trace.startTimer('search');
    let searchOutcome;
    const primaryName = this.searchProvider instanceof GoogleSearchProvider ? 'google' : 'searxng';
    const cacheKey = plannedQueries.map((q) => q.query).join(';');
    const cachedHits = this.config.cache.enabled
      ? await this.searchCache.get(primaryName, cacheKey, route.freshness)
      : null;

    if (cachedHits && cachedHits.length > 0) {
      searchOutcome = { results: cachedHits, rawCount: cachedHits.length, failureCount: 0 };
      trace.update({ providerUsed: primaryName, fallbackUsed: false, fallbackReason: '' });
    } else {
      const searchService = new SearchService(this.searchProvider, {
        maxConcurrentQueries: this.config.maxConcurrentQueries,
        retries: this.config.searchRetries,
        retryDelayMs: this.config.searchRetryDelayMs,
      });
      searchOutcome = await searchService.executeSearches(
        plannedQueries,
        this.config.resultsPerQuery
      );
      trace.update({
        queryFailures: searchOutcome.failures,
        searchFailureCount: searchOutcome.failureCount,
      });
      if (this.searchProvider instanceof SearXNGProvider && this.searchProvider.lastDiagnostics) {
        trace.update({ searxngDiagnostics: { ...this.searchProvider.lastDiagnostics } });
      }
      if (this.config.cache.enabled && searchOutcome.results.length > 0) {
        await this.searchCache.set(
          primaryName,
          cacheKey,
          searchOutcome.results,
          route.freshness,
          ttlForFreshness(route.freshness, this.config.cache.searchTtlSeconds.default)
        );
      }
      trace.update({ providerUsed: primaryName, fallbackUsed: false, fallbackReason: '' });
      // Automatic fallback: primary empty + fallback configured + quota remains.
      if (searchOutcome.results.length === 0 && this.fallbackProvider) {
        const fallbackName = this.fallbackProvider instanceof GoogleSearchProvider ? 'google' : 'searxng';
        let reason = '';
        if (fallbackName === 'google' && !(await this.claimGoogleQuota(plannedQueries.length))) {
          reason = 'google daily quota exhausted';
        } else {
          const fbService = new SearchService(this.fallbackProvider);
          const fbOutcome = await fbService.executeSearches(plannedQueries, this.config.resultsPerQuery);
          if (fbOutcome.results.length > 0) {
            searchOutcome = fbOutcome;
            reason = `primary ${primaryName} empty; fell back to ${fallbackName}`;
            if (this.config.cache.enabled) {
              await this.searchCache.set(fallbackName, cacheKey, fbOutcome.results, route.freshness, this.config.cache.searchTtlSeconds.default);
            }
          } else {
            reason = `primary ${primaryName} and fallback ${fallbackName} both empty`;
          }
        }
        trace.update({ providerUsed: fallbackName, fallbackUsed: searchOutcome.results.length > 0, fallbackReason: reason });
        this.logger.log('search_fallback', { primary: primaryName, fallback: fallbackName, reason });
      }
    }
    trace.endTimer('search');
    trace.update({
      searchResults: searchOutcome.rawCount,
      uniqueResults: searchOutcome.results.length,
    });
    this.logger.log('search_completed', {
      raw: searchOutcome.rawCount,
      unique: searchOutcome.results.length,
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
        this.logger.log('search_pagination', { pages: pagedPages, recovered: fused.length });
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
    const pageBudget = mode === 'fast' ? this.config.fetch.fastPages : mode === 'deep' ? this.config.fetch.deepPages : this.config.fetch.normalPages;
    const pagesToFetch = selectPagesToFetch(rankedResults, mode, this.config.fetch).slice(0, Math.max(1, pageBudget - retryReserve * 2));
    let pagesAttempted = pagesToFetch.length;
    trace.update({ pagesSelected: pagesToFetch.length });
    this.logger.log('pages_selected', { count: pagesToFetch.length });

    // Step 6: Safe Page Fetching & Main Content Extraction
    trace.startTimer('fetch_extract');
    const documents: RetrievedDocument[] = [];
    let fetchFailures = 0;
    let successfulFetches = 0;
    let extractedTokens = 0;

    const fetchPromises = pagesToFetch.map(async (candidate, index) => {
      // Check document cache
      if (this.config.cache.enabled) {
        const cached = await this.documentCache.get(candidate.url);
        if (cached) { successfulFetches++; return { ...cached, id: `doc-${index + 1}` }; }
      }

      const fetchRes = await this.fetcher.fetch(candidate.url, {
        timeoutSeconds: this.config.fetch.timeoutSeconds,
        maxBytes: this.config.fetch.maxBytes,
        userAgent: this.config.fetch.userAgent,
      });

      let doc: RetrievedDocument | null = null;

      if (!fetchRes.success || !fetchRes.body || fetchRes.body.length < 150) {
        fetchFailures++;
        // Section 115 & Section 26: If page fetch fails or yields insufficient body, fall back to snippet evidence
        if (candidate.snippet && candidate.snippet.trim().length > 0) {
          const fallbackExt = fallbackSnippetExtraction(candidate.title, candidate.snippet, candidate.url);
          doc = {
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
            metadata: { extractionMethod: 'search_snippet', fetchError: fetchRes.error },
          };
        }
      } else {
        successfulFetches++;
        const extraction = extractMainContent(fetchRes.body, candidate.title);
        doc = {
          id: `doc-${index + 1}`,
          url: fetchRes.finalUrl || candidate.url,
          canonicalUrl: extraction.canonicalUrl || candidate.canonicalUrl,
          domain: candidate.domain,
          title: extraction.title || candidate.title,
          author: extraction.author,
          publishedAt: extraction.publishedAt || candidate.publishedAt,
          text: extraction.text,
          headings: extraction.headings,
          links: extraction.links,
          contentHash: extraction.contentHash,
          retrievedAt: new Date().toISOString(),
          searchResultIds: [candidate.id],
          metadata: { extractionMethod: 'main_content' },
        };
      }

      if (doc && this.config.cache.enabled && fetchRes.success) {
        await this.documentCache.set(doc, route.freshness === 'any' ? 86400 : 900);
      }

      return doc;
    });

    const settledDocs = await Promise.allSettled(fetchPromises);
    for (const res of settledDocs) {
      if (res.status === 'fulfilled' && res.value) {
        extractedTokens += defaultTokenCounter.count(res.value.text);
        if (!documents.some(d => d.contentHash && d.contentHash === res.value?.contentHash)) documents.push(res.value);
      } else if (res.status === 'rejected') {
        fetchFailures++;
      }
    }
    trace.endTimer('fetch_extract');
    trace.update({
      pagesFetched: successfulFetches,
      fetchFailures,
      extractedTokens,
    });
    this.logger.log('fetch_completed', { fetched: documents.length, failures: fetchFailures });

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

    // Step 8: Hybrid Retrieval (BM25 + Dense Embeddings + RRF + Diversity)
    trace.startTimer('retrieval');
    const hybridRetriever = new HybridRetriever(this.embeddingProvider, {
      bm25Enabled: this.config.retrieval.bm25,
      embeddingsEnabled: this.config.retrieval.embeddings,
      rrfK: this.config.retrieval.rrfK,
      maxChunksPerDoc: this.config.retrieval.maxChunksPerDoc,
    });
    const candidateChunks = await hybridRetriever.retrieve(
      question,
      allChunks,
      this.config.retrieval.candidateLimit
    );
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
      retryCount < this.config.verification.maxResearchRetries && plannedQueries.length < totalQueryBudget && pagesAttempted < pageBudget
    ) {
      const retryEval = evaluateResearchRetry(
        verificationReport,
        retryCount,
        this.config.verification.maxResearchRetries,
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
      this.logger.log('retry_search_initiated', {
        round: retryCount,
        targetQuery: retryEval.targetedQuery,
      });

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

        const retryPages = selectPagesToFetch(newRanked, 'fast', this.config.fetch).filter(r => !documents.some(d => d.url === r.url)).slice(0, Math.min(2, pageBudget - pagesAttempted));
        pagesAttempted += retryPages.length;
        rankedResults.push(...newRanked.filter(r => !rankedResults.some(old => old.url === r.url)));
        const retryChunks: EvidenceChunk[] = [];

        for (const candidate of retryPages) {
          let retryDoc: RetrievedDocument | null = null;
          const fetchRes = await this.fetcher.fetch(candidate.url, {
            timeoutSeconds: this.config.fetch.timeoutSeconds,
            maxBytes: this.config.fetch.maxBytes,
          });

          if (fetchRes.success && fetchRes.body && fetchRes.body.length >= 150) {
            successfulFetches++;
            const ext = extractMainContent(fetchRes.body, candidate.title);
            retryDoc = {
              id: `doc-${crypto.randomUUID()}`,
              url: fetchRes.finalUrl || candidate.url,
              canonicalUrl: ext.canonicalUrl || candidate.canonicalUrl,
              domain: candidate.domain,
              title: ext.title || candidate.title,
              text: ext.text,
              headings: ext.headings,
              links: ext.links,
              contentHash: ext.contentHash,
              retrievedAt: new Date().toISOString(),
              searchResultIds: [candidate.id],
            };
          } else if (candidate.snippet && candidate.snippet.trim().length > 0) {
            fetchFailures++;
            const ext = fallbackSnippetExtraction(candidate.title, candidate.snippet, candidate.url);
            retryDoc = {
              id: `doc-${crypto.randomUUID()}`,
              url: candidate.url,
              domain: candidate.domain,
              title: candidate.title,
              text: ext.text,
              contentHash: ext.contentHash,
              retrievedAt: new Date().toISOString(),
              searchResultIds: [candidate.id],
            };
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
          const retryCandidateChunks = await hybridRetriever.retrieve(
            retryEval.targetedQuery.query,
            retryChunks,
            5
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
          const roundStats = recordRound(researchState, retryEval.targetedQuery.query, updatedFinalClaims, sourceManager.getAllSources());
          if (roundStats.newEvidence === 0) {
            this.logger.log('retry_search_initiated', { round: retryCount, stagnant: true });
          }
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

