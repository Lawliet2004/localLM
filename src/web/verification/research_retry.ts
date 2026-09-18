/**
 * Research Retry: Evaluates verification reports and decides whether to trigger
 * a targeted search round for unsupported or missing factual claims.
 *
 * Stagnation guard: repeated identical targeted queries, or rounds with no new
 * evidence, stop the loop instead of burning more budget.
 */

import type { VerificationReport, PlannedQuery } from '../types';

export interface RetryDecision {
  shouldRetry: boolean;
  targetedQuery?: PlannedQuery;
  reason?: string;
}

/** Normalize a query for repeat detection. */
export function normalizeQueryText(query: string): string {
  return query.toLowerCase().replace(/[^\w\s]/g, '').split(/\s+/).filter(Boolean).sort().join(' ');
}

export function isRepeatedQuery(query: string, tried: string[]): boolean {
  const key = normalizeQueryText(query);
  return tried.some((t) => normalizeQueryText(t) === key);
}

export function evaluateResearchRetry(
  verification: VerificationReport,
  currentRetryCount: number,
  maxRetries: number = 1,
  triedQueries: string[] = [],
  roundsWithoutNewEvidence = 0,
): RetryDecision {
  if (currentRetryCount >= maxRetries) {
    return { shouldRetry: false, reason: 'Retry budget exhausted.' };
  }
  if (roundsWithoutNewEvidence >= 2) {
    return { shouldRetry: false, reason: 'No new evidence in recent rounds; stopping instead of repeating.' };
  }

  const unsupported = verification.claims.filter((c) => c.status === 'UNSUPPORTED');
  if (unsupported.length === 0) {
    return { shouldRetry: false, reason: 'All claims adequately supported.' };
  }

  // Select the most critical unsupported claim
  const targetClaim = unsupported[0].claim;

  // Derive a targeted keyword query from the claim
  const keywords = targetClaim
    .replace(/[^\w\s]/g, '')
    .split(/\s+/)
    .filter((w) => w.length > 3)
    .slice(0, 6)
    .join(' ');

  if (isRepeatedQuery(keywords, triedQueries)) {
    return { shouldRetry: false, reason: 'Targeted query already tried; stopping instead of repeating.' };
  }

  return {
    shouldRetry: true,
    targetedQuery: {
      query: keywords,
      purpose: `Verify unsupported claim: "${targetClaim.slice(0, 80)}"`,
      freshness: 'any',
    },
    reason: `Found ${unsupported.length} unsupported claim(s). Initiating targeted research.`,
  };
}
