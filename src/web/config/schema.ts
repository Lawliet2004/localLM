/**
 * Configuration schema for the web search and grounding engine.
 */

import type { ResearchMode } from '../types';

export interface WebSearchFetchConfig {
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
  maxConcurrentQueries: number;
  searchRetries: number;
  searchRetryDelayMs: number;
  fetch: WebSearchFetchConfig;
  cache: WebSearchCacheConfig;
}
