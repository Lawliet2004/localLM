/**
 * Configuration schema for the web search and grounding engine.
 */

import type { ResearchMode } from '../types';

export interface WebSearchRankingConfig {
  semanticWeight: number;
  lexicalWeight: number;
  freshnessWeight: number;
  authorityWeight: number;
  queryCoverageWeight: number;
  searchRankWeight: number;
}

export interface WebSearchFetchConfig {
  fastPages: number;
  normalPages: number;
  deepPages: number;
  timeoutSeconds: number;
  maxBytes: number;
  globalConcurrency: number;
  perDomainConcurrency: number;
  userAgent: string;
  /**
   * Retry failed page fetches through the Wayback Machine before falling back
   * to snippet-only evidence. Snapshot provenance is marked in document text
   * and metadata. Only triggered on failure, so it costs nothing when pages load.
   */
  waybackFallback: boolean;
  /** Learn per-domain fetch success rates and skip chronic failing domains. */
  domainLearning: boolean;
  /**
   * Optional last-resort headless-browser render (Edge `--dump-dom`) for
   * challenge interstitials and JS-dependent pages whose static extraction is
   * too thin to use. Runs inside the research worker on desktop only.
   */
  jsRenderFallback: boolean;
  /** Hard timeout for the render subprocess, milliseconds. */
  jsRenderTimeoutMs: number;
}

export interface WebSearchChunkingConfig {
  targetTokens: number;
  overlapTokens: number;
}

export interface WebSearchRetrievalConfig {
  candidateLimit: number;
  finalLimit: number;
  maxChunksPerDoc: number;
}

export interface WebSearchContextConfig {
  totalInputBudget: number;
  evidenceTokenBudget: number;
  systemBudget: number;
  questionBudget: number;
  safetyMargin: number;
}

export interface WebSearchVerificationConfig {
  enabled: boolean;
  maxResearchRetries: number;
}

export interface WebSearchCacheConfig {
  enabled: boolean;
  searchTtlSeconds: number;
}

/**
 * One research run's spend limits for a mode (fast/normal/deep). This is the
 * single source of truth — the query planner, staged research loop, dynamic
 * follow-ups, page fetching, and the verification retry loop all read these
 * instead of keeping their own hardcoded clamps.
 */
export interface ResearchBudget {
  /** Queries planned up front for one research question. */
  initialQueries: number;
  /** Dynamically added follow-up or cross-check queries on top of the initial set. */
  followUpQueries: number;
  /** Total page fetches per run across every query and retry round. */
  pageFetches: number;
  /** Verification retry rounds after the initial pass (verification.enabled still gates them). */
  retries: number;
  /** Wall-clock cap for one run in milliseconds; callers may still pass a tighter deadlineMs. */
  timeMs: number;
}

export interface WebSearchConfig {
  extractEvidenceWithModel: boolean;
  enabled: boolean;
  mode: ResearchMode;
  searxngBaseUrl: string;
  searxngTimeoutMs: number;
  searxngEngines: string[];
  searxngDisabledEngines: string[];
  searchProvider: 'searxng' | 'google';
  googleApiKey: string;
  googleCxId: string;
  searchFallback: {
    enabled: boolean;
    googleDailyLimit: number;
  };
  queries: {
    fast: number;
    normal: number;
    deep: number;
  };
  maxConcurrentQueries: number;
  searchRetries: number;
  searchRetryDelayMs: number;
  enablePagination: boolean;
  resultsPerQuery: number;
  fetch: WebSearchFetchConfig;
  chunking: WebSearchChunkingConfig;
  retrieval: WebSearchRetrievalConfig;
  reranking: {
    enabled: boolean;
    /**
     * Blends published-date recency into rerank scores: exp decay with a
     * 90-day half-life scale, added as weight * recencyScore. Applied only
     * when the route has no explicit freshness window (the window already
     * filters for recency at the search stage).
     */
    recencyWeight: number;
  };
  ranking: WebSearchRankingConfig;
  context: WebSearchContextConfig;
  verification: WebSearchVerificationConfig;
  cache: WebSearchCacheConfig;
}
