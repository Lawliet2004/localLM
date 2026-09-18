/**
 * Fetch Policy: Selects candidate search results to fetch based on research mode,
 * score ranking, and URL safety.
 */

import type { SearchResult, ResearchMode } from '../types';
import type { WebSearchFetchConfig } from '../config/schema';
import { isSafeUrl } from '../security/ssrf_guard';

export function selectPagesToFetch(
  results: SearchResult[],
  mode: ResearchMode,
  config: WebSearchFetchConfig
): SearchResult[] {
  let targetCount = config.normalPages;
  if (mode === 'fast') targetCount = config.fastPages;
  if (mode === 'deep') targetCount = config.deepPages;

  const validCandidates = results.filter((r) => isSafeUrl(r.url));
  return validCandidates.slice(0, targetCount);
}
