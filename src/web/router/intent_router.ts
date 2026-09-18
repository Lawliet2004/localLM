/**
 * Intent Router: Routes incoming user queries to appropriate execution pipelines:
 * static knowledge, structured data verticals (Weather, Currency, Time), or Web Search.
 */

import type { RouteResult } from '../types';
import { detectFreshnessRequirement } from './freshness';
import { routeVertical } from './vertical_router';

export function routeRequest(question: string): RouteResult {
  const trimmed = question.trim();
  const freshness = detectFreshnessRequirement(trimmed);
  const vertical = routeVertical(trimmed);

  // If query is static knowledge
  if (vertical === 'NONE') {
    return {
      requiresExternalData: false,
      requiresWebSearch: false,
      vertical: 'NONE',
      freshness: 'any',
      confidence: 0.95,
      complexity: 'simple',
      reasoning: 'Static conceptual knowledge query does not require external data.',
    };
  }

  // If structured vertical (Weather, Currency, Time)
  const isStructuredVertical = ['WEATHER', 'CURRENCY', 'TIME'].includes(vertical);
  if (isStructuredVertical) {
    return {
      requiresExternalData: true,
      requiresWebSearch: false, // Handled by zero-cost structured API
      vertical,
      freshness: vertical === 'WEATHER' || vertical === 'TIME' ? 'realtime' : freshness,
      confidence: 0.98,
      complexity: 'simple',
      reasoning: `Routed to structured vertical: ${vertical}`,
    };
  }

  // Complexity estimation
  let complexity: 'simple' | 'medium' | 'complex' = 'medium';
  const lower = trimmed.toLowerCase();

  if (/\b(compare|versus|vs\.?|differences between|benchmark comparison)\b/i.test(lower)) {
    complexity = 'complex';
  } else if (/\b(what is the (version|capital|population|height)|who is)\b/i.test(lower) && trimmed.length < 50) {
    complexity = 'simple';
  }

  return {
    requiresExternalData: true,
    requiresWebSearch: true,
    vertical,
    freshness,
    confidence: 0.92,
    complexity,
    reasoning: `Requires web search in vertical ${vertical} with freshness ${freshness}`,
  };
}
