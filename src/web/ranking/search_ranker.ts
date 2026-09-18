/**
 * Search Ranker: Scores and ranks candidate search results using multi-factor signals:
 * lexical match, freshness decay, source authority, query coverage, and search rank prior.
 */

import type { SearchResult, FreshnessWindow, VerticalType } from '../types';
import type { WebSearchRankingConfig } from '../config/schema';
import { assessSourceQuality } from './source_quality';
import { calculateFreshnessScore } from './freshness_score';

export interface RankOptions {
  query: string;
  freshness: FreshnessWindow;
  vertical: VerticalType;
  totalPlannedQueries: number;
  weights: WebSearchRankingConfig;
  currentDate?: Date;
}

export function rankSearchResults(
  results: SearchResult[],
  options: RankOptions
): SearchResult[] {
  const queryWords = options.query
    .toLowerCase()
    .replace(/[^\w\s]/g, '')
    .split(/\s+/)
    .filter((w) => w.length > 2);

  const scored = results.map((result) => {
    const titleLower = result.title.toLowerCase();
    const snippetLower = (result.snippet || '').toLowerCase();
    const textToMatch = `${titleLower} ${snippetLower}`;

    // 1. Lexical match score (fraction of query keywords found)
    let matchedKeywords = 0;
    for (const word of queryWords) {
      if (textToMatch.includes(word)) {
        matchedKeywords++;
      }
    }
    const lexicalScore = queryWords.length > 0 ? matchedKeywords / queryWords.length : 0.5;

    // Title bonus: exact title match or keywords in title
    const titleKeywords = queryWords.filter((w) => titleLower.includes(w)).length;
    const titleBonus = queryWords.length > 0 ? (titleKeywords / queryWords.length) * 0.2 : 0;
    const effectiveLexical = Math.min(1.0, lexicalScore + titleBonus);

    // 2. Authority score from domain / source quality
    const { authorityScore } = assessSourceQuality(result.url, result.title);

    // 3. Freshness score with intent decay
    const freshnessScore = calculateFreshnessScore(
      result.publishedAt,
      options.freshness,
      options.vertical,
      options.currentDate
    );

    // 4. Query coverage (if multiple queries were fused, how many matched)
    const distinctQueryIds = result.queryId ? result.queryId.split(',').length : 1;
    const queryCoverageScore = Math.min(
      1.0,
      distinctQueryIds / Math.max(1, options.totalPlannedQueries)
    );

    // 5. Search rank prior (1.0 for rank 1, decaying for lower ranks)
    const rankPrior = 1.0 / (1.0 + Math.log2(Math.max(1, result.rank)));

    // Combined weighted score
    const w = options.weights;
    const score =
      w.lexicalWeight * effectiveLexical +
      w.semanticWeight * effectiveLexical + // semantic proxy from snippet
      w.authorityWeight * authorityScore +
      w.freshnessWeight * freshnessScore +
      w.queryCoverageWeight * queryCoverageScore +
      w.searchRankWeight * rankPrior;

    return {
      ...result,
      score: Number(score.toFixed(4)),
    };
  });

  // Sort descending by score
  return scored.sort((a, b) => (b.score ?? 0) - (a.score ?? 0));
}
