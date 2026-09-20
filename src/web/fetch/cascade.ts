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

export type FetchMethod = 'github_raw' | 'live' | 'wayback' | 'failed';

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
        await record(false, `${CHALLENGE_MISS_REASON}:${challenge.marker}`);
      } else {
        await record(true);
        return {
          document: fromHtml(live.body, titleHint, live.finalUrl || url, 'live'),
          raw: live,
          method: 'live',
        };
      }
    } else {
      await record(false, live.error);
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
    error: skippedLive ? 'chronic domain skipped live fetch' : 'fetch failed',
  };
}
