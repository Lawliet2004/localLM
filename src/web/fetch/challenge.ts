/**
 * Cloudflare-style challenge detection.
 *
 * A Managed Challenge / Turnstile interstitial is often served with HTTP 200,
 * so a "successful" fetch can be a bot wall. Treating that as real content
 * poisons evidence for small local models and teaches domain-learning that
 * the host is healthy. Scope is narrow: identify challenges, not low-quality
 * pages. Over-matching honest prose is worse than missing a wall.
 */

export type ChallengeKind = 'status_headers' | 'interstitial_body';

export interface ChallengeSignal {
  kind: ChallengeKind;
  marker: string;
}

export const CHALLENGE_MISS_REASON = 'challenge_detected';

const BODY_SCAN_LIMIT = 64 * 1024;
const ERROR_PAGE_SCAN_LIMIT = 8 * 1024;
const ERROR_PAGE_HEADING = /^(?:sorry[,!\s]+)?(?:(?:the\s+)?page\s+not\s+found|(?:the\s+)?requested\s+(?:page|url|resource)\s+(?:was\s+)?not\s+found|404(?:\s+(?:error|not found))?|not found|access denied|403 forbidden)(?:\s*[-|:]\s*[\w .-]{1,60})?$/i;

const BODY_MARKERS: readonly { marker: string; re: RegExp }[] = [
  { marker: 'title:just-a-moment', re: /<title[^>]*>\s*just a moment/i },
  { marker: 'cf-chl', re: /cf-chl/i },
  { marker: 'challenge-platform', re: /challenge-platform/i },
  { marker: '_cf_chl_opt', re: /_cf_chl_opt/i },
];

/** Detect obvious HTTP error templates served with a misleading 200 status. */
export function detectErrorPage(body?: string | null): string | null {
  if (!body) return null;
  const head = body.slice(0, ERROR_PAGE_SCAN_LIMIT);
  const headings = [...head.matchAll(/<(title|h1)\b[^>]*>([\s\S]*?)<\/\1\s*>/gi)];
  for (const match of headings) {
    const text = match[2].replace(/<[^>]*>/g, ' ').replace(/&nbsp;|&#160;/gi, ' ').replace(/\s+/g, ' ').trim();
    if (ERROR_PAGE_HEADING.test(text)) {
      return `error_page:${match[1].toLowerCase()}`;
    }
  }
  return null;
}

export function detectChallenge(
  status: number,
  body?: string | null,
): ChallengeSignal | null {
  if (status === 403 || status === 503) {
    if (body) {
      const head = body.slice(0, BODY_SCAN_LIMIT);
      for (const { marker, re } of BODY_MARKERS) {
        if (re.test(head)) return { kind: 'status_headers', marker: `status_${status}:${marker}` };
      }
    }
  }
  if (body) {
    const head = body.slice(0, BODY_SCAN_LIMIT);
    for (const { marker, re } of BODY_MARKERS) {
      if (re.test(head)) return { kind: 'interstitial_body', marker };
    }
  }
  return null;
}
