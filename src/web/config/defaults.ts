/**
 * Default configurations for the web search engine.
 */

import type { WebSearchConfig } from './schema';

export const DEFAULT_CONFIG: WebSearchConfig = {
  enabled: true,
  mode: 'normal',
  profile: 'standard',
  searxngBaseUrl: 'http://127.0.0.1:8080',
  // Allow for the instance's outgoing request timeout and response processing.
  searxngTimeoutMs: 15000,
  // Let each self-hosted instance manage its engine catalogue and credentials.
  searxngEngines: [],
  searxngDisabledEngines: [],
  searchProvider: 'searxng',
  googleApiKey: '',
  googleCxId: '',
  searchFallback: {
    enabled: true,
    googleDailyLimit: 90,
  },
  maxConcurrentQueries: 4,
  searchRetries: 1,
  searchRetryDelayMs: 1000,
  fetch: {
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
 * Environment overrides, applied last so a deployment can pin the SearXNG
 * endpoint and engine catalogue without touching app settings (the app always
 * injects a base URL, so env has to win to be useful):
 * SEARXNG_BASE_URL, SEARXNG_TIMEOUT_MS, SEARXNG_ENGINES (csv),
 * SEARXNG_DISABLED_ENGINES (csv), AGC_WEB_PROFILE (=low_memory).
 */
function envOverrides(): Partial<WebSearchConfig> {
  const env = (globalThis as { process?: { env?: Record<string, string | undefined> } }).process?.env;
  if (!env) return {};
  const out: Partial<WebSearchConfig> = {};
  const list = (value?: string) => value?.split(',').map((s) => s.trim()).filter(Boolean) ?? [];
  if (env.SEARXNG_BASE_URL?.trim()) out.searxngBaseUrl = env.SEARXNG_BASE_URL.trim();
  const timeout = Number(env.SEARXNG_TIMEOUT_MS);
  if (Number.isFinite(timeout) && timeout >= 1000 && timeout <= 120000) out.searxngTimeoutMs = Math.floor(timeout);
  const engines = list(env.SEARXNG_ENGINES);
  if (engines.length) out.searxngEngines = engines;
  const disabled = list(env.SEARXNG_DISABLED_ENGINES);
  if (disabled.length) out.searxngDisabledEngines = disabled;
  if (env.AGC_WEB_PROFILE === 'low_memory') out.profile = 'low_memory';
  return out;
}

export function createConfig(overrides?: Partial<WebSearchConfig>): WebSearchConfig {
  const env = envOverrides();

  const base: WebSearchConfig = {
    ...DEFAULT_CONFIG,
    ...overrides,
    ...env,
  };
  // Merge nested settings without sharing mutable defaults between engine instances.
  for (const key of ['fetch','cache','searchFallback'] as const) {
    Object.assign(base, {[key]: {...DEFAULT_CONFIG[key], ...overrides?.[key]}});
  }
  if (base.profile === 'low_memory') {
    return {
      ...base,
      maxConcurrentQueries: Math.min(base.maxConcurrentQueries, 2),
      fetch: {
        ...base.fetch,
        globalConcurrency: 4,
        perDomainConcurrency: 1,
      },
    };
  }

  return base;
}
