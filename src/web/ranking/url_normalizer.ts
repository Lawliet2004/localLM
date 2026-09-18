/**
 * URL Normalizer: Normalizes URLs for canonical deduplication while preserving
 * meaningful identifying parameters and removing tracking/marketing parameters.
 */

const TRACKING_PARAMS = new Set([
  'utm_source',
  'utm_medium',
  'utm_campaign',
  'utm_term',
  'utm_content',
  'utm_id',
  'fbclid',
  'gclid',
  'gclsrc',
  'dclid',
  'msclkid',
  'mc_eid',
  'mc_cid',
  '_hsenc',
  '_hsmi',
  'yclid',
  'twclid',
  'igshid',
]);

export function normalizeUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl.trim());

    // Lowercase protocol and host
    url.protocol = url.protocol.toLowerCase();
    url.hostname = url.hostname.toLowerCase();

    // Standardize default ports
    if ((url.protocol === 'http:' && url.port === '80') || (url.protocol === 'https:' && url.port === '443')) {
      url.port = '';
    }

    // Strip www. prefix for consistent comparison
    if (url.hostname.startsWith('www.')) {
      url.hostname = url.hostname.substring(4);
    }

    // Strip fragment/hash
    url.hash = '';

    // Filter tracking query parameters
    const searchParams = new URLSearchParams(url.search);
    const keysToDelete: string[] = [];

    for (const key of searchParams.keys()) {
      const lowerKey = key.toLowerCase();
      if (TRACKING_PARAMS.has(lowerKey) || lowerKey.startsWith('utm_')) {
        keysToDelete.push(key);
      }
    }

    for (const key of keysToDelete) {
      searchParams.delete(key);
    }

    // Sort remaining parameters for deterministic order
    const sortedKeys = Array.from(new Set(searchParams.keys())).sort();
    const sortedParams = new URLSearchParams();
    for (const k of sortedKeys) {
      const values = searchParams.getAll(k);
      for (const v of values) {
        sortedParams.append(k, v);
      }
    }

    const queryStr = sortedParams.toString();
    url.search = queryStr ? `?${queryStr}` : '';

    // Remove trailing slash on path (except root /)
    let path = url.pathname;
    if (path.length > 1 && path.endsWith('/')) {
      path = path.slice(0, -1);
    }
    url.pathname = path;

    return url.toString();
  } catch {
    // If URL is unparseable, return trimmed
    return rawUrl.trim();
  }
}

/**
 * Extracts a normalized domain hostname.
 */
export function extractDomain(urlStr: string): string {
  try {
    const url = new URL(urlStr);
    let host = url.hostname.toLowerCase();
    if (host.startsWith('www.')) {
      host = host.substring(4);
    }
    return host;
  } catch {
    return 'unknown';
  }
}
