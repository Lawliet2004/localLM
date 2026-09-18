/**
 * Freshness Detector: Identifies temporal intent from user query keywords and expressions.
 */

import type { FreshnessWindow } from '../types';

export function detectFreshnessRequirement(query: string): FreshnessWindow {
  const q = query.toLowerCase();

  // Realtime expressions
  if (/\b(right now|live|current temperature|realtime|at this moment)\b/i.test(q)) {
    return 'realtime';
  }

  // Day expressions
  if (/\b(today|tonight|this morning|yesterday|past 24 hours)\b/i.test(q)) {
    return 'day';
  }

  // Week expressions
  if (/\b(this week|past week|last few days|past 7 days)\b/i.test(q)) {
    return 'week';
  }

  // Month / Recent expressions
  if (/\b(latest|newest|recent|recently|this month|new release|new version|changelog)\b/i.test(q)) {
    return 'month';
  }

  // Year expressions
  if (/\b(this year|2025|2026|2024|annual)\b/i.test(q)) {
    return 'year';
  }

  return 'any';
}
