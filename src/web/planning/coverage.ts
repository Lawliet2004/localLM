import type { EvidenceClaim } from '../types';
export interface CoverageCell { entity: string; dimension: string; covered: boolean }
export function comparisonEntities(question: string): string[] {
  if (!/\b(compare|versus|vs\.?)\b/i.test(question)) return [];
  const match = question.match(/(?:compare\s+(?:.*?\bfor\s+)?)(.+?)\s+(?:and|versus|vs\.?)\s+(.+?)(?:\s+(?:using|on|in terms of)\b|[?!.]|$)/i)
    || question.match(/(.+?)\s+(?:versus|vs\.?)\s+(.+?)(?:[?!.]|$)/i);
  return match ? [match[1],match[2]].map(s => s.trim()).filter(s => s.length > 1 && s.length < 100) : [];
}
export function coverageMatrix(question: string, evidence: EvidenceClaim[]): CoverageCell[] {
  const entities = comparisonEntities(question);
  const dimensions = [
    ['benchmark', /benchmark|humaneval|swe.bench|accuracy|score|performance/i],
    ['parameters', /parameters|\b\d+(?:\.\d+)?\s*[bm]\b/i],
    ['license', /licen[sc]e|apache|mit|commercial/i],
    ['release', /release|launch|version/i],
  ] as const;
  const needed = dimensions.filter(([_, pattern]) => pattern.test(question));
  return entities.flatMap(entity => (needed.length ? needed : [dimensions[0]]).map(([dimension, pattern]) => ({
    entity, dimension, covered: evidence.some(e => e.claim.toLowerCase().includes(entity.toLowerCase()) && pattern.test(e.claim)),
  })));
}
