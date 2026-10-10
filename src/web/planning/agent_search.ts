import type { LLMProvider } from '../types';
import { researchBudget } from '../config/defaults';
import { detectFreshnessRequirement } from '../router/freshness';
import { resolveQuestion } from '../staged_research';
import { compactQuery, localDateString } from './fallback_planner';
import { QueryPlanner } from './query_planner';

/** Prepare AREX search calls without changing its search/visit tool contract. */
export async function planAgentSearch(input: {
  queries: string[];
  context?: string;
  model?: LLMProvider;
  currentDate?: string;
}): Promise<{ queries: string[]; clarificationQuestion?: string }> {
  const date = input.currentDate ?? localDateString();
  const year = date.slice(0, 4);
  // The AREX tool schema caps callers at 4 queries per call — the input
  // contract — but query expansion follows the central research budget.
  const limit = researchBudget().initialQueries;
  const raw = input.queries.slice(0, 4).map(query => query.trim()).filter(Boolean);
  if (!raw.length) return { queries: [] };
  const resolved: string[] = [];
  for (const query of raw) {
    const result = await resolveQuestion(query, input.context, input.model);
    if (result.clarificationQuestion) return { queries: [], clarificationQuestion: result.clarificationQuestion };
    resolved.push(result.question);
  }
  if (resolved.length === 1) {
    const freshness = detectFreshnessRequirement(resolved[0]);
    const planned = await new QueryPlanner(input.model).planQueries(resolved[0], freshness, limit, date);
    return { queries: planned.map(item => item.query) };
  }
  const queries = resolved.map(query => compactQuery(query, /\b(latest|current|newest|now|today|recent)\b/i.test(query), year));
  return { queries: [...new Set(queries)].slice(0, limit) };
}
