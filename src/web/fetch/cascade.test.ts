import { expect, it } from 'vitest';
import { fetchPageCascade } from './cascade';
import { InMemoryStorageAdapter } from '../cache/sqlite';
import { DomainStatsStore } from './domain_stats';

it('uses the GitHub raw README instead of the JS shell', async () => {
  const fetcher = {
    fetch: async (url: string) => {
      if (url.includes('raw.githubusercontent.com') && url.endsWith('/HEAD/README.md')) {
        return {
          url,
          finalUrl: url,
          success: true,
          status: 200,
          mimeType: 'text/plain',
          body: '# Project\n\nThis README is the real repository documentation body.',
          durationMs: 1,
        };
      }
      throw new Error(`unexpected fetch ${url}`);
    },
  };
  const outcome = await fetchPageCascade('https://github.com/owner/repo', fetcher, { waybackFallback: false });
  expect(outcome.method).toBe('github_raw');
  expect(outcome.document?.text).toContain('real repository documentation');
});

it('treats a Cloudflare interstitial as a miss and tries Wayback', async () => {
  const fetcher = {
    fetch: async (url: string) => {
      if (url.includes('archive.org/wayback/available')) {
        return {
          url,
          finalUrl: url,
          success: true,
          status: 200,
          body: JSON.stringify({
            archived_snapshots: {
              closest: {
                available: true,
                url: 'https://web.archive.org/web/20240101000000/https://blocked.example/page',
                timestamp: '20240101000000',
              },
            },
          }),
          durationMs: 1,
        };
      }
      if (url.includes('web.archive.org')) {
        return {
          url,
          finalUrl: url,
          success: true,
          status: 200,
          body: '<html><article><p>Archived copy of the real article with enough text to extract after the live Cloudflare interstitial was rejected as a miss rather than as content.</p></article></html>',
          durationMs: 1,
        };
      }
      return {
        url,
        finalUrl: url,
        success: true,
        status: 200,
        body: '<html><title>Just a moment...</title><div id="challenge-platform"></div></html>',
        durationMs: 1,
      };
    },
  };
  const outcome = await fetchPageCascade('https://blocked.example/page', fetcher);
  expect(outcome.method).toBe('wayback');
  expect(outcome.document?.text).toContain('Archived copy');
});

it('rescues a challenge interstitial through the JS render fallback', async () => {
  const fetcher = {
    fetch: async (url: string) => ({
      url,
      finalUrl: url,
      success: true,
      status: 200,
      body: '<html><title>Just a moment...</title><div id="challenge-platform"></div></html>',
      durationMs: 1,
    }),
  };
  const outcome = await fetchPageCascade('https://blocked.example/page', fetcher, {
    waybackFallback: false,
    jsRender: async (url) => ({
      html: '<html><article><p>Rendered article text that only appears after scripts run in a real browser engine.</p></article></html>',
      finalUrl: url,
    }),
  });
  expect(outcome.method).toBe('js_render');
  expect(outcome.document?.text).toContain('Rendered article text');
});

it('renders thin pages but keeps the richer extraction', async () => {
  const thin = '<html><body><div id="app">Loading application shell…</div><script src="app.js"></script></body></html>'.padEnd(200, ' ');
  const fetcher = {
    fetch: async (url: string) => ({ url, finalUrl: url, success: true, status: 200, body: thin, durationMs: 1 }),
  };
  const richer = await fetchPageCascade('https://spa.example/', fetcher, {
    waybackFallback: false,
    jsRender: async () => ({
      html: '<html><article><p>The SPA rendered this substantial paragraph only in a browser, giving the cascade enough text to keep.</p></article></html>',
    }),
  });
  expect(richer.method).toBe('js_render');
  // A render that produces even less text than the thin live extraction is not an upgrade.
  const poorer = await fetchPageCascade('https://spa.example/', fetcher, {
    waybackFallback: false,
    jsRender: async () => ({ html: '<html><body><p>.</p></body></html>' }),
  });
  expect(poorer.method).toBe('live');
  expect(poorer.document?.text).toContain('Loading application shell');
});

it('never renders a page that robots.txt disallowed', async () => {
  const fetcher = {
    fetch: async (url: string) => ({ url, finalUrl: url, success: false, error: 'robots.txt disallows this page', durationMs: 1 }),
  };
  let rendered = 0;
  const outcome = await fetchPageCascade('https://private.example/page', fetcher, {
    waybackFallback: false,
    jsRender: async () => { rendered++; return { html: '<html><article><p>should not be fetched</p></article></html>' }; },
  });
  expect(outcome.method).toBe('failed');
  expect(rendered).toBe(0);
});

it('tries the renderer on chronic domains and heals their stats', async () => {
  const stats = new DomainStatsStore(new InMemoryStorageAdapter(), { minAttempts: 4, chronicFailRate: 0.7 });
  for (let i = 0; i < 4; i++) await stats.record('walled.example', false, 'challenge_detected');
  const outcome = await fetchPageCascade('https://walled.example/page', { fetch: async () => { throw new Error('unreachable'); } }, {
    waybackFallback: false,
    domainStats: stats,
    jsRender: async (url) => ({
      html: '<html><article><p>Browser-rendered content recovered for a domain that static fetches always fail on.</p></article></html>',
      finalUrl: url,
    }),
  });
  expect(outcome.method).toBe('js_render');
  expect((await stats.status('walled.example')).ok).toBe(1);
});

it('skips live fetches for chronic domains', async () => {
  const stats = new DomainStatsStore(new InMemoryStorageAdapter(), { minAttempts: 4, chronicFailRate: 0.7 });
  for (let i = 0; i < 4; i++) await stats.record('walled.example', false, '403');
  const calls: string[] = [];
  const fetcher = {
    fetch: async (url: string) => {
      calls.push(url);
      return { url, finalUrl: url, success: false, error: 'blocked', durationMs: 1 };
    },
  };
  const outcome = await fetchPageCascade('https://walled.example/page', fetcher, {
    waybackFallback: false,
    domainStats: stats,
  });
  expect(outcome.skippedLive).toBe(true);
  expect(outcome.method).toBe('failed');
  expect(calls.some((u) => u === 'https://walled.example/page')).toBe(false);
});
