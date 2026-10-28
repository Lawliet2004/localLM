import { afterEach, describe, expect, it } from 'vitest';
import { createConfig, DEFAULT_CONFIG } from './defaults';

const ENV_KEYS = ['SEARXNG_BASE_URL', 'SEARXNG_TIMEOUT_MS', 'SEARXNG_ENGINES', 'SEARXNG_DISABLED_ENGINES', 'AGC_WEB_PROFILE'];
const saved: Record<string, string | undefined> = {};
for (const key of ENV_KEYS) saved[key] = process.env[key];

afterEach(() => {
  for (const key of ENV_KEYS) {
    if (saved[key] === undefined) delete process.env[key];
    else process.env[key] = saved[key];
  }
});

describe('createConfig environment overrides', () => {
  it('applies SEARXNG_* env vars over defaults and caller config', () => {
    process.env.SEARXNG_BASE_URL = 'http://127.0.0.1:9090';
    process.env.SEARXNG_TIMEOUT_MS = '5000';
    process.env.SEARXNG_ENGINES = 'duckduckgo, wikipedia';
    process.env.SEARXNG_DISABLED_ENGINES = 'yep';
    const config = createConfig({ searxngBaseUrl: 'http://127.0.0.1:8080' });
    expect(config.searxngBaseUrl).toBe('http://127.0.0.1:9090');
    expect(config.searxngTimeoutMs).toBe(5000);
    expect(config.searxngEngines).toEqual(['duckduckgo', 'wikipedia']);
    expect(config.searxngDisabledEngines).toEqual(['yep']);
  });

  it('ignores malformed env values', () => {
    process.env.SEARXNG_TIMEOUT_MS = 'not-a-number';
    process.env.SEARXNG_BASE_URL = '   ';
    const config = createConfig();
    expect(config.searxngTimeoutMs).toBe(DEFAULT_CONFIG.searxngTimeoutMs);
    expect(config.searxngBaseUrl).toBe(DEFAULT_CONFIG.searxngBaseUrl);
  });

  it('keeps low-memory limits scoped to retrieval transport', () => {
    process.env.AGC_WEB_PROFILE = 'low_memory';
    const config = createConfig();
    expect(config.maxConcurrentQueries).toBe(2);
    expect(config.fetch.globalConcurrency).toBe(4);
    expect(config.fetch.perDomainConcurrency).toBe(1);
    expect(config.fetch.maxBytes).toBe(DEFAULT_CONFIG.fetch.maxBytes);
    config.fetch.userAgent = 'isolated';
    expect(DEFAULT_CONFIG.fetch.userAgent).not.toBe('isolated');
  });
});
