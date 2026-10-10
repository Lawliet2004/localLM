/**
 * Default configurations for the web search engine.
 */

import type { ResearchBudget, WebSearchConfig } from './schema';
import type { ResearchMode } from '../types';

/** Per-mode research spend limits — the only place these numbers live. */
export const RESEARCH_BUDGETS: Record<ResearchMode, ResearchBudget> = {
  fast: { initialQueries: 2, followUpQueries: 2, pageFetches: 4, retries: 0, timeMs: 30_000 },
  normal: { initialQueries: 4, followUpQueries: 2, pageFetches: 8, retries: 1, timeMs: 90_000 },
  deep: { initialQueries: 6, followUpQueries: 4, pageFetches: 20, retries: 2, timeMs: 180_000 },
};

export function researchBudget(mode?: ResearchMode): ResearchBudget {
  return RESEARCH_BUDGETS[mode ?? 'normal'];
}

export const DEFAULT_CONFIG: WebSearchConfig = {
  extractEvidenceWithModel: false,
  enabled: true,
  mode: 'normal',
  searxngBaseUrl: 'http://127.0.0.1:8080',
  // Fan-out to ~11 engines under parallel AREX queries regularly exceeds 8s;
  // a 15s budget keeps whole query batches from timing out together.
  searxngTimeoutMs: 15000,
  searxngEngines: ['google cse', 'yep', 'duckduckgo', 'google news', 'reuters', 'wikipedia', 'wikinews', 'stackoverflow', 'github', 'arxiv', 'openstreetmap'],
  searxngDisabledEngines: ['google', 'bing', 'yandex', 'brave', 'qwant'],
  searchProvider: 'searxng',
  googleApiKey: '',
  googleCxId: '',
  searchFallback: {
    enabled: true,
    googleDailyLimit: 90,
  },
  queries: {
    fast: 2,
    normal: 4,
    deep: 6,
  },
  maxConcurrentQueries: 4,
  searchRetries: 1,
  searchRetryDelayMs: 1000,
  enablePagination: true,
  resultsPerQuery: 10,
  fetch: {
    fastPages: 4,
    normalPages: 8,
    deepPages: 20,
    timeoutSeconds: 10,
    maxBytes: 5 * 1024 * 1024, // 5MB
    globalConcurrency: 8,
    perDomainConcurrency: 2,
    userAgent: 'LocalLM-Research/1.0 (+https://github.com/locallm/desktop)',
    waybackFallback: true,
    domainLearning: true,
    jsRenderFallback: false,
    jsRenderTimeoutMs: 20000,
  },
  chunking: {
    targetTokens: 600,
    overlapTokens: 80,
  },
  retrieval: {
    candidateLimit: 25,
    finalLimit: 8,
    maxChunksPerDoc: 2,
  },
  reranking: {
    enabled: false,
    recencyWeight: 0.15,
  },
  ranking: {
    semanticWeight: 0.35,
    lexicalWeight: 0.20,
    freshnessWeight: 0.15,
    authorityWeight: 0.15,
    queryCoverageWeight: 0.10,
    searchRankWeight: 0.05,
  },
  context: {
    totalInputBudget: 6000,
    evidenceTokenBudget: 2500,
    systemBudget: 800,
    questionBudget: 400,
    safetyMargin: 400,
  },
  verification: {
    enabled: false,
    maxResearchRetries: 1,
  },
  cache: {
    enabled: true,
    searchTtlSeconds: 86400, // 24 hours
  },
};

export function createConfig(overrides?: Partial<WebSearchConfig>): WebSearchConfig {
  const base: WebSearchConfig = {
    ...DEFAULT_CONFIG,
    ...overrides,
  };
  // Merge nested settings without sharing mutable defaults between engine instances.
  for (const key of ['queries','fetch','chunking','retrieval','reranking','ranking','context','verification','cache','searchFallback'] as const) {
    Object.assign(base, {[key]: {...DEFAULT_CONFIG[key], ...overrides?.[key]}});
  }
  return base;
}
