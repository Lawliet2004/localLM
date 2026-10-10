/**
 * Fetch Policy: Selects candidate search results to fetch based on research mode,
 * score ranking, and URL safety.
 */

import type { SearchResult } from '../types';
import { isSafeUrl } from '../security/ssrf_guard';

export function selectPagesToFetch(
  results: SearchResult[],
  maxPages: number
): SearchResult[] {
  const validCandidates = results.filter((r) => isSafeUrl(r.url));
  return validCandidates.slice(0, maxPages);
}
