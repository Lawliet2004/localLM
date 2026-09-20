/**
 * Wayback Machine fallback: recovers a failed page fetch from the Internet
 * Archive. Only consulted after a live fetch fails or returns no usable body,
 * so it costs nothing on healthy pages. The CDX availability lookup and the
 * snapshot download both travel through the same pinned, SSRF-checked fetcher;
 * archive.org permits automated fetching (its robots.txt only disallows
 * /control/ and /report/) and web.archive.org publishes no robots.txt.
 */

import { extractMainContent } from '../extraction/main_content';

const CDX_URL = 'https://archive.org/wayback/available';
const CDX_TIMEOUT_MS = 8000;

export interface WaybackSnapshot {
  snapshotUrl: string;
  archivedAt: string;
}

export interface WaybackRecovery {
  /** Original URL that was requested. */
  originalUrl: string;
  title: string;
  text: string;
  snapshotUrl: string;
  archivedAt: string;
}

function isHttpUrl(url: string): boolean {
  try {
    return /^https?:$/.test(new URL(url).protocol);
  } catch {
    return false;
  }
}

/** Look up the closest archived snapshot for a URL. Returns null on any miss. */
export async function queryWaybackSnapshot(
  url: string,
  fetcher: Pick<HttpFetcherLike, 'fetch'>,
): Promise<WaybackSnapshot | null> {
  if (!isHttpUrl(url)) return null;
  try {
    const res = await fetcher.fetch(`${CDX_URL}?url=${encodeURIComponent(url)}`, {
      timeoutSeconds: Math.ceil(CDX_TIMEOUT_MS / 1000),
      maxBytes: 256 * 1024,
    });
    if (!res.success || !res.body) return null;
    const data = JSON.parse(res.body) as {
      archived_snapshots?: { closest?: { url?: string; timestamp?: string; available?: boolean } };
    };
    const closest = data.archived_snapshots?.closest;
    if (!closest?.url || closest.available === false) return null;
    // Defensive: only follow snapshot URLs that point back at the archive.
    if (!closest.url.startsWith('https://web.archive.org/')) return null;
    const ts = closest.timestamp ?? '';
    const archivedAt = /^\d{8}/.test(ts)
      ? `${ts.slice(0, 4)}-${ts.slice(4, 6)}-${ts.slice(6, 8)}`
      : 'unknown date';
    return { snapshotUrl: closest.url, archivedAt };
  } catch {
    return null;
  }
}

export interface HttpFetcherLike {
  fetch(url: string, options?: FetchOptionsLike): Promise<FetchResultLike>;
}

export interface FetchOptionsLike { timeoutSeconds?: number; maxBytes?: number; maxRedirects?: number; userAgent?: string }
export interface FetchResultLike { url: string; finalUrl: string; success: boolean; status?: number; body?: string; mimeType?: string; error?: string; durationMs: number }

/**
 * Attempt full recovery of a URL from the Wayback Machine: snapshot lookup,
 * bounded download, main-content extraction, and an explicit provenance line
 * prepended to the text so downstream synthesis knows this is an archived copy.
 */
export async function fetchViaWayback(
  originalUrl: string,
  fetcher: Pick<HttpFetcherLike, 'fetch'>,
  options: { timeoutSeconds?: number; maxBytes?: number; titleHint?: string } = {},
): Promise<WaybackRecovery | null> {
  const snapshot = await queryWaybackSnapshot(originalUrl, fetcher);
  if (!snapshot) return null;
  try {
    const res = await fetcher.fetch(snapshot.snapshotUrl, {
      timeoutSeconds: options.timeoutSeconds,
      maxBytes: options.maxBytes,
    });
    if (!res.success || !res.body || res.body.length < 150) return null;
    const extraction = extractMainContent(res.body, options.titleHint || originalUrl);
    if (!extraction.text || extraction.text.trim().length < 50) return null;
    const provenance = `> [via Wayback Machine, archived ${snapshot.archivedAt}] — live page was unavailable\n\n`;
    return {
      originalUrl,
      title: `[Archived] ${extraction.title || options.titleHint || originalUrl}`,
      text: provenance + extraction.text,
      snapshotUrl: res.finalUrl || snapshot.snapshotUrl,
      archivedAt: snapshot.archivedAt,
    };
  } catch {
    return null;
  }
}
