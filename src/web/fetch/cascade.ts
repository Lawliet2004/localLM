/**
 * In-process fetch cascade modeled on self-hosted SearXNG stacks:
 * GitHub raw content → live HTML fetch → Wayback Machine.
 *
 * No headless browser, Firecrawl, or Crawl4AI: those need extra containers
 * and RAM that a local-model desktop harness should not require. Each stage
 * is optional and fail-soft. Challenge interstitials are treated as misses.
 */

import { extractMainContent, computeContentHash } from '../extraction/main_content';
import type { FetchOptions, FetchResult } from './http_fetcher';
import { toGitHubRawCandidates, isUsableRawResponse } from './github_fast_path';
import { fetchViaWayback } from './wayback';
import { detectChallenge, CHALLENGE_MISS_REASON } from './challenge';
import type { DomainStatsStore } from './domain_stats';

export type FetchMethod = 'github_raw' | 'live' | 'js_render' | 'wayback' | 'failed';

/** Rendered-DOM provider (headless Edge in the desktop worker). */
export type JsRenderer = (url: string) => Promise<{ html: string; finalUrl?: string } | null>;

export interface CascadeDocument {
  title: string;
  text: string;
  finalUrl: string;
  method: FetchMethod;
  publishedAt?: string;
  author?: string;
  canonicalUrl?: string;
  headings?: string[];
  links?: Array<{ text: string; href: string }>;
  contentHash: string;
  archivedAt?: string;
}

export interface CascadeOutcome {
  document: CascadeDocument | null;
  raw?: FetchResult;
  method: FetchMethod;
  error?: string;
  challenge?: boolean;
  skippedLive?: boolean;
}

export interface CascadeOptions {
  timeoutSeconds?: number;
  maxBytes?: number;
  userAgent?: string;
  titleHint?: string;
  waybackFallback?: boolean;
  domainStats?: DomainStatsStore;
  /**
   * Optional headless-render fallback for pages the static fetch cannot read:
   * challenge interstitials, failed live fetches, and extractions thinner than
   * jsRenderMinChars. Not attempted when robots.txt disallows the page.
   */
  jsRender?: JsRenderer;
  /** Extracted text shorter than this marks the page JS-dependent. */
  jsRenderMinChars?: number;
}

function domainOf(url: string): string {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return '';
  }
}

function isPdf(res: FetchResult, url: string): boolean {
  const mime = (res.mimeType || '').split(';')[0].trim().toLowerCase();
  return mime === 'application/pdf' || /\.pdf(\?|#|$)/i.test(res.finalUrl || url);
}

function fromHtml(body: string, titleHint: string, finalUrl: string, method: FetchMethod): CascadeDocument {
  const extraction = extractMainContent(body, titleHint);
  return {
    title: extraction.title || titleHint,
    text: extraction.text,
    finalUrl,
    method,
    publishedAt: extraction.publishedAt,
    author: extraction.author,
    canonicalUrl: extraction.canonicalUrl,
    headings: extraction.headings,
    links: extraction.links,
    contentHash: extraction.contentHash,
  };
}

export async function fetchPageCascade(
  url: string,
  fetcher: { fetch(url: string, options?: FetchOptions): Promise<FetchResult> },
  options: CascadeOptions = {},
): Promise<CascadeOutcome> {
  const domain = domainOf(url);
  const fetchOpts: FetchOptions = {
    timeoutSeconds: options.timeoutSeconds,
    maxBytes: options.maxBytes,
    userAgent: options.userAgent,
  };
  const titleHint = options.titleHint || url;

  let skippedLive = false;
  if (options.domainStats && domain) {
    const status = await options.domainStats.status(domain);
    if (status.chronic) skippedLive = true;
  }

  const record = async (ok: boolean, error?: string) => {
    if (options.domainStats && domain) await options.domainStats.record(domain, ok, error);
  };

  // The real failure reason (HTTP status, robots refusal, challenge marker)
  // reaches the caller — a bare "fetch failed" teaches the model nothing and
  // invites blind retries.
  let failureDetail: string | undefined;

  // Rendered output is untrusted page markup; it goes through the same
  // extraction and challenge checks as a static body before it counts.
  const tryRender = async (): Promise<CascadeOutcome | null> => {
    if (!options.jsRender) return null;
    const rendered = await options.jsRender(url).catch(() => null);
    if (!rendered?.html || detectChallenge(200, rendered.html)) return null;
    const document = fromHtml(rendered.html, titleHint, rendered.finalUrl || url, 'js_render');
    if (!document.text.trim()) return null;
    return { document, method: 'js_render' };
  };

  const minUsefulChars = options.jsRenderMinChars ?? 280;

  if (!skippedLive) {
    const github = toGitHubRawCandidates(url);
    if (github) {
      for (const candidate of github) {
        const res = await fetcher.fetch(candidate.rawUrl, fetchOpts);
        if (isUsableRawResponse(res.status, res.mimeType, res.body, candidate.kind)) {
          await record(true);
          return {
            document: fromHtml(res.body!, titleHint, res.finalUrl || candidate.rawUrl, 'github_raw'),
            raw: res,
            method: 'github_raw',
          };
        }
      }
    }

    const live = await fetcher.fetch(url, fetchOpts);
    if (live.success && live.body && live.body.length >= 150) {
      if (isPdf(live, url)) {
        await record(true);
        return { document: null, raw: live, method: 'live' };
      }
      const challenge = detectChallenge(live.status ?? 200, live.body);
      if (challenge) {
        failureDetail = `${CHALLENGE_MISS_REASON}:${challenge.marker}`;
        await record(false, failureDetail);
        const rendered = await tryRender();
        if (rendered) {
          await record(true);
          return rendered;
        }
      } else {
        const document = fromHtml(live.body, titleHint, live.finalUrl || url, 'live');
        if (options.jsRender && document.text.trim().length < minUsefulChars) {
          const rendered = await tryRender();
          if (rendered && rendered.document!.text.trim().length > document.text.trim().length) {
            await record(true);
            return rendered;
          }
        }
        await record(true);
        return { document, raw: live, method: 'live' };
      }
    } else {
      failureDetail = live.error || 'empty or thin response';
      await record(false, live.error);
      // A browser render can outlive plain HTTP failures (TLS fingerprints,
      // transient errors) but must never bypass a robots.txt refusal.
      if (!/robots\.txt/i.test(live.error || '')) {
        const rendered = await tryRender();
        if (rendered) {
          await record(true);
          return rendered;
        }
      }
    }
  } else {
    // Chronic static-fetch failures are usually bot walls; a render attempt is
    // the one mechanism that can still reach the page and heal the domain.
    const rendered = await tryRender();
    if (rendered) {
      await record(true);
      return rendered;
    }
  }

  if (options.waybackFallback !== false) {
    const recovered = await fetchViaWayback(url, fetcher, {
      timeoutSeconds: options.timeoutSeconds,
      maxBytes: options.maxBytes,
      titleHint,
    });
    if (recovered) {
      return {
        document: {
          title: recovered.title,
          text: recovered.text,
          finalUrl: recovered.snapshotUrl,
          method: 'wayback',
          contentHash: computeContentHash(recovered.text),
          archivedAt: recovered.archivedAt,
        },
        method: 'wayback',
        skippedLive,
      };
    }
  }

  return {
    document: null,
    method: 'failed',
    skippedLive,
    error: skippedLive ? 'chronic domain skipped live fetch' : (failureDetail || 'fetch failed'),
  };
}
