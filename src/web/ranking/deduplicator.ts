/**
 * Result Deduplicator: Deduplicates search results using normalized URL equality,
 * canonical URL matching, and domain + title similarity (token Jaccard / Levenshtein).
 */

import type { SearchResult } from '../types';
import { normalizeUrl } from './url_normalizer';

/**
 * Computes Token Jaccard similarity between two strings.
 */
export function tokenJaccardSimilarity(a: string, b: string): number {
  const tokenize = (s: string): Set<string> =>
    new Set(
      s
        .toLowerCase()
        .replace(/[^\w\s]/g, '')
        .split(/\s+/)
        .filter((t) => t.length > 1)
    );

  const setA = tokenize(a);
  const setB = tokenize(b);

  if (setA.size === 0 && setB.size === 0) return 1.0;
  if (setA.size === 0 || setB.size === 0) return 0.0;

  let intersectionSize = 0;
  for (const token of setA) {
    if (setB.has(token)) {
      intersectionSize++;
    }
  }

  const unionSize = setA.size + setB.size - intersectionSize;
  return unionSize === 0 ? 0 : intersectionSize / unionSize;
}

/**
 * Computes normalized Levenshtein similarity [0.0 - 1.0].
 */
export function levenshteinSimilarity(a: string, b: string): number {
  const s1 = a.toLowerCase().trim();
  const s2 = b.toLowerCase().trim();

  if (s1 === s2) return 1.0;
  if (s1.length === 0 || s2.length === 0) return 0.0;

  const m = s1.length;
  const n = s2.length;
  const dp: number[][] = Array.from({ length: m + 1 }, () => new Array(n + 1).fill(0));

  for (let i = 0; i <= m; i++) dp[i][0] = i;
  for (let j = 0; j <= n; j++) dp[0][j] = j;

  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      const cost = s1[i - 1] === s2[j - 1] ? 0 : 1;
      dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + cost);
    }
  }

  const distance = dp[m][n];
  const maxLength = Math.max(m, n);
  return 1 - distance / maxLength;
}

export function deduplicateSearchResults(results: SearchResult[]): SearchResult[] {
  const seenUrls = new Map<string, SearchResult>();
  const uniqueResults: SearchResult[] = [];

  for (const result of results) {
    const normUrl = normalizeUrl(result.url);
    const normCanonical = result.canonicalUrl ? normalizeUrl(result.canonicalUrl) : undefined;

    // Check exact normalized URL or canonical URL
    const existing = seenUrls.get(normUrl) || (normCanonical ? seenUrls.get(normCanonical) : undefined);
    if (existing) {
      // Merge query IDs or metadata if from different queries
      if (result.queryId && existing.queryId && !existing.queryId.includes(result.queryId)) {
        existing.queryId = `${existing.queryId},${result.queryId}`;
      }
      continue;
    }

    // Check same domain + high title similarity OR syndicated news across domains
    let isDuplicate = false;
    for (const u of uniqueResults) {
      const jaccard = tokenJaccardSimilarity(u.title, result.title);
      // Same domain duplicate
      if (u.domain === result.domain && jaccard > 0.75) {
        isDuplicate = true;
        if (result.queryId && u.queryId && !u.queryId.includes(result.queryId)) {
          u.queryId = `${u.queryId},${result.queryId}`;
        }
        break;
      }
      // Syndicated content across different domains (e.g. AP/Reuters syndicated articles)
      if (u.domain !== result.domain && (jaccard >= 0.88 || levenshteinSimilarity(u.title, result.title) >= 0.90)) {
        isDuplicate = true;
        if (result.queryId && u.queryId && !u.queryId.includes(result.queryId)) {
          u.queryId = `${u.queryId},${result.queryId}`;
        }
        break;
      }
    }

    if (!isDuplicate) {
      seenUrls.set(normUrl, result);
      if (normCanonical) {
        seenUrls.set(normCanonical, result);
      }
      uniqueResults.push(result);
    }
  }

  return uniqueResults;
}
