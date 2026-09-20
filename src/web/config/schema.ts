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
  deepTimeoutSeconds: number;
  maxBytes: number;
  deepMaxBytes: number;
  globalConcurrency: number;
  perDomainConcurrency: number;
  userAgent: string;
  userAgents: string[];
  /**
   * Retry failed page fetches through the Wayback Machine before falling back
   * to snippet-only evidence. Snapshot provenance is marked in document text
   * and metadata. Only triggered on failure, so it costs nothing when pages load.
   */
  waybackFallback: boolean;
  /** Learn per-domain fetch success rates and skip chronic failing domains. */
  domainLearning: boolean;
}

export interface WebSearchChunkingConfig {
  targetTokens: number;
  overlapTokens: number;
}

export interface WebSearchRetrievalConfig {
  bm25: boolean;
  embeddings: boolean;
  rrfK: number;
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
  databasePath?: string;
  searchTtlSeconds: {
    news: number;
    weather: number;
    documentation: number;
    default: number;
  };
}

export interface WebSearchConfig {
  extractEvidenceWithModel: boolean;
  localEmbedding?: { baseUrl: string; model: string };
  localReranker?: { baseUrl: string; model: string };
  enabled: boolean;
  mode: ResearchMode;
  profile: 'standard' | 'low_memory';
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
