import { describe, it, expect } from 'vitest';
import { calculateFreshnessScore } from './freshness_score';

describe('Freshness Scoring', () => {
  const baseDate = new Date('2026-09-16T12:00:00Z');

  it('decays rapidly for realtime / news queries', () => {
    const today = '2026-09-16T08:00:00Z'; // 4 hours old
    const weekOld = '2026-09-09T12:00:00Z'; // 7 days old

    const scoreToday = calculateFreshnessScore(today, 'day', 'NEWS', baseDate);
    const scoreWeekOld = calculateFreshnessScore(weekOld, 'day', 'NEWS', baseDate);

    expect(scoreToday).toBeGreaterThan(0.8);
    expect(scoreWeekOld).toBeLessThan(0.3);
  });

  it('decays slowly for evergreen / any queries', () => {
    const oneYearAgo = '2025-09-16T12:00:00Z';
    const score = calculateFreshnessScore(oneYearAgo, 'any', 'NONE', baseDate);
    expect(score).toBeGreaterThan(0.6);
  });

  it('assigns sensible default prior when date is missing', () => {
    const score = calculateFreshnessScore(undefined, 'month', 'GENERAL_WEB', baseDate);
    expect(score).toBe(0.6);
  });
});
