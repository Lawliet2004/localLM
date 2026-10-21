import { describe, expect, it } from 'vitest';
import type { ProviderConnection, RemoteModel } from './types';
import { isLoopbackBaseUrl, isSubscriptionFormat, parseProviderBaseUrl, providerRequiresApiKey, remoteModelReady } from './providers';

const provider: ProviderConnection = {
  id: 'p1', name: 'Remote', baseUrl: 'https://api.example.com/v1', apiFormat: 'openai-chat-completions',
  hasApiKey: true, verified: true, lastTestedAt: null, models: [],
};
const model: RemoteModel = { id: 'm1', contextLength: 8192, maxOutputTokens: null, toolSupport: 'unknown', supportsImages: false };

describe('provider endpoints', () => {
  it('normalizes API prefixes and accepts actual loopback hosts, including IPv6', () => {
    expect(parseProviderBaseUrl(' https://api.example.com/v1/ ')).toEqual({ normalized: 'https://api.example.com/v1', loopback: false });
    for (const host of ['localhost', '127.0.0.1', '[::1]']) {
      expect(parseProviderBaseUrl(`http://${host}:8080/v1`)).toEqual({ normalized: `http://${host}:8080/v1`, loopback: true });
    }
  });

  it('rejects malformed, insecure, or credential-bearing URLs and query/fragment delimiters', () => {
    for (const url of ['', 'not-a-url', 'http://api.example.com', 'https://user:password@api.example.com',
      'https://api.example.com?key=value', 'https://api.example.com#fragment', 'https://api.example.com?', 'https://api.example.com#']) {
      expect(parseProviderBaseUrl(url), url).toHaveProperty('error');
    }
  });

  it('does not mistake host substrings or path segments for loopback addresses', () => {
    expect(isLoopbackBaseUrl('http://[::1]:8080')).toBe(true);
    for (const url of ['https://localhost.evil.example', 'https://127.0.0.1.evil.example', 'https://api.example.com/::1', 'https://api.example.com/localhost', 'bogus']) {
      expect(isLoopbackBaseUrl(url), url).toBe(false);
    }
  });
});

describe('provider readiness', () => {
  it('requires keys for hosted endpoints and all native Claude endpoints', () => {
    expect(providerRequiresApiKey('openai-chat-completions', 'https://api.example.com')).toBe(false);
    expect(providerRequiresApiKey('openai-chat-completions', 'http://127.0.0.1:8080')).toBe(false);
    for (const url of ['https://api.example.com', 'http://localhost:8080', 'http://[::1]:8080']) {
      expect(providerRequiresApiKey('claude-messages', url), url).toBe(true);
    }
  });

  it('requires verification, context capacity, a valid endpoint, and required credentials', () => {
    expect(remoteModelReady(provider, model)).toBe(true);
    expect(remoteModelReady({ ...provider, verified: false }, model)).toBe(false);
    expect(remoteModelReady(provider, { ...model, contextLength: null })).toBe(false);
    // Keys are optional for OpenAI-compatible providers — keyless is ready.
    expect(remoteModelReady({ ...provider, hasApiKey: false }, model)).toBe(true);
    expect(remoteModelReady({ ...provider, baseUrl: 'http://api.example.com' }, model)).toBe(false);
    expect(remoteModelReady({ ...provider, baseUrl: 'https://localhost.evil.example', hasApiKey: false }, model)).toBe(true);
    expect(remoteModelReady({ ...provider, baseUrl: 'http://[::1]:8080', hasApiKey: false }, model)).toBe(true);
    expect(remoteModelReady({ ...provider, baseUrl: 'http://[::1]:8080', apiFormat: 'claude-messages', hasApiKey: false }, model)).toBe(false);
  });

  it('keeps hosted subscriptions unavailable while allowing explicit custom API models', () => {
    for (const apiFormat of ['chatgpt-subscription', 'grok-subscription'] as const) {
      expect(isSubscriptionFormat(apiFormat)).toBe(true);
      expect(remoteModelReady({ ...provider, apiFormat }, model)).toBe(false);
    }
    expect(isSubscriptionFormat('claude-messages')).toBe(false);
    expect(remoteModelReady({ ...provider, apiFormat: 'claude-messages' }, model)).toBe(true);
  });
});

it('only offers effort control for documented families and dated snapshots', async () => {
  const { supportsReasoningEffort } = await import('./providers');
  expect(supportsReasoningEffort('gpt-5')).toBe(true);
  expect(supportsReasoningEffort('o3-mini')).toBe(true);
  expect(supportsReasoningEffort('gpt-5-2025-08-07')).toBe(true);
  expect(supportsReasoningEffort('gpt-5-pro')).toBe(false);
  expect(supportsReasoningEffort('gpt-5-chat-latest')).toBe(false);
  expect(supportsReasoningEffort('custom-model')).toBe(false);
});
