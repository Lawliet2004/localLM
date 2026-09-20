/**
 * Default configurations for the web search engine.
 */

import type { WebSearchConfig } from './schema';

export const DEFAULT_CONFIG: WebSearchConfig = {
  extractEvidenceWithModel: false,
  enabled: true,
  mode: 'normal',
  profile: 'standard',
  searxngBaseUrl: 'http://127.0.0.1:8080',
  searxngTimeoutMs: 8000,
  searxngEngines: ['duckduckgo', 'wikipedia', 'stackoverflow', 'github', 'arxiv', 'openstreetmap'],
  searxngDisabledEngines: ['google', 'bing', 'yandex', 'brave', 'mojeek'],
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
    deepTimeoutSeconds: 20,
    maxBytes: 5 * 1024 * 1024, // 5MB
    deepMaxBytes: 10 * 1024 * 1024, // 10MB
    globalConcurrency: 8,
    perDomainConcurrency: 2,
    userAgent: 'LocalLM-Research/1.0 (+https://github.com/locallm/desktop)',
    waybackFallback: true,
    domainLearning: true,
    userAgents: [
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0',
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
      'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
    ],
  },
  chunking: {
    targetTokens: 600,
    overlapTokens: 80,
  },
  retrieval: {
    bm25: true,
    embeddings: false,
    rrfK: 60,
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
    databasePath: './data/web-cache.sqlite',
    searchTtlSeconds: {
      news: 3600, // 1 hour
      weather: 1800, // 30 minutes
      documentation: 86400 * 7, // 7 days
      default: 86400, // 24 hours
    },
  },
};

/**
 * Creates configuration adjusted for the low_memory profile.
 */
export function createConfig(overrides?: Partial<WebSearchConfig>): WebSearchConfig {
  const envProfile =
    typeof (globalThis as { process?: { env?: Record<string, string | undefined> } }).process !== 'undefined' &&
    (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env?.AGC_WEB_PROFILE === 'low_memory'
      ? 'low_memory'
      : undefined;

  const base: WebSearchConfig = {
    ...DEFAULT_CONFIG,
    ...(envProfile ? { profile: 'low_memory' } : {}),
    ...overrides,
  };
  // Merge nested settings without sharing mutable defaults between engine instances.
  for (const key of ['queries','fetch','chunking','retrieval','reranking','ranking','context','verification','cache','searchFallback'] as const) {
    Object.assign(base, {[key]: {...DEFAULT_CONFIG[key], ...overrides?.[key]}});
  }
  if (base.profile === 'low_memory') {
    return {
      ...base,
      maxConcurrentQueries: Math.min(base.maxConcurrentQueries, 2),
      fetch: {
        ...base.fetch,
        fastPages: 3,
        normalPages: 4,
        deepPages: 8,
        globalConcurrency: 4,
        perDomainConcurrency: 1,
      },
      chunking: {
        targetTokens: 400,
        overlapTokens: 50,
      },
      retrieval: {
        ...base.retrieval,
        candidateLimit: 15,
        finalLimit: 5,
        maxChunksPerDoc: 2,
      },
      reranking: {
        ...base.reranking,
        enabled: false, // disable cross-encoder on low RAM
      },
      context: {
        totalInputBudget: 4096,
        evidenceTokenBudget: 1800,
        systemBudget: 600,
        questionBudget: 300,
        safetyMargin: 300,
      },
    };
  }

  return base;
}
