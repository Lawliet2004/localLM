/**
 * Freshness Scoring: Intent-dependent decay function to prioritize recent results
 * when freshness matters (news, weather, latest releases) while not penalizing evergreen facts.
 */

import type { FreshnessWindow, VerticalType } from '../types';

export function calculateFreshnessScore(
  publishedAt: string | undefined,
  freshnessReq: FreshnessWindow,
  vertical: VerticalType,
  currentDate: Date = new Date()
): number {
  if (!publishedAt) {
    // If no date is available, assign a neutral prior based on freshness requirement
    if (freshnessReq === 'realtime' || freshnessReq === 'day') return 0.35;
    if (freshnessReq === 'week') return 0.50;
    if (freshnessReq === 'month') return 0.60;
    return 0.70;
  }

  const pubDate = new Date(publishedAt);
  if (isNaN(pubDate.getTime())) {
    return 0.50;
  }

  const ageMs = Math.max(0, currentDate.getTime() - pubDate.getTime());
  const ageHours = ageMs / (1000 * 60 * 60);
  const ageDays = ageHours / 24;

  // Determine half-life in days based on freshness requirement and vertical
  let halfLifeDays = 30; // default 1 month

  if (freshnessReq === 'realtime' || vertical === 'WEATHER') {
    halfLifeDays = 0.5; // 12 hours
  } else if (freshnessReq === 'day' || vertical === 'NEWS') {
    halfLifeDays = 2; // 2 days
  } else if (freshnessReq === 'week') {
    halfLifeDays = 7;
  } else if (freshnessReq === 'month' || vertical === 'DOCUMENTATION') {
    halfLifeDays = 60;
  } else if (freshnessReq === 'year') {
    halfLifeDays = 365;
  } else if (freshnessReq === 'any') {
    halfLifeDays = 1000; // very slow decay
  }

  // Exponential decay formula: score = e^(-ln(2) * age / halfLife)
  const score = Math.exp((-Math.LN2 * ageDays) / halfLifeDays);
  return Math.min(1.0, Math.max(0.05, score));
}
