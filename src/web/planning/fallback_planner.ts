/**
 * Fallback Query Planner: Generates targeted, non-overlapping search queries
 * deterministically without requiring generative LLM calls.
 */

import type { PlannedQuery, FreshnessWindow } from '../types';

export function planQueriesDeterministically(
  question: string,
  freshness: FreshnessWindow = 'any',
  maxQueries: number = 4
): PlannedQuery[] {
  const queries: PlannedQuery[] = [];
  const cleanQ = question.trim();

  // Query 1: Direct question
  queries.push({
    query: cleanQ,
    purpose: 'Direct overview search for question intent',
    freshness,
  });

  if (maxQueries <= 1) return queries;

  const lower = cleanQ.toLowerCase();

  // Comparison detection: "Compare X and Y" or "X vs Y"
  const vsMatch = lower.match(/(?:compare\s+)?([^,]+?)\s+(?:and|vs\.?|versus)\s+([^,]+)/i);
  if (vsMatch) {
    const entityA = vsMatch[1].replace(/^(what is|compare)/i, '').trim();
    const entityB = vsMatch[2].trim();

    if (queries.length < maxQueries && entityA.length > 2) {
      queries.push({
        query: `${entityA} official specs benchmark`,
        purpose: `Primary specs and benchmark data for ${entityA}`,
        freshness,
      });
    }

    if (queries.length < maxQueries && entityB.length > 2) {
      queries.push({
        query: `${entityB} official specs benchmark`,
        purpose: `Primary specs and benchmark data for ${entityB}`,
        freshness,
      });
    }
  }

  // Software / Release notes detection
  if (/\b(release|version|changed|newest|latest|update)\b/i.test(lower) && queries.length < maxQueries) {
    queries.push({
      query: `${cleanQ} official changelog release notes`,
      purpose: 'Official release notes and changelog verification',
      freshness: 'month',
    });
  }

  // Model / Benchmark detection
  if (/\b(model|benchmark|coding|swe-bench|eval)\b/i.test(lower) && queries.length < maxQueries) {
    queries.push({
      query: `${cleanQ} model card benchmark results`,
      purpose: 'Technical model card and benchmark evaluation',
      freshness: 'month',
    });
  }

  return queries.slice(0, maxQueries);
}
