/**
 * Query Planner: Generates minimal, targeted search queries using local LLM
 * inference or deterministic fallback.
 */

import type { PlannedQuery, FreshnessWindow, LLMProvider } from '../types';
import { planQueriesDeterministically } from './fallback_planner';
import { QUERY_PLAN_SCHEMA } from './query_schema';

export class QueryPlanner {
  constructor(private llmProvider?: LLMProvider) {}

  async planQueries(
    question: string,
    freshness: FreshnessWindow = 'any',
    maxQueries: number = 4,
    currentDate?: string
  ): Promise<PlannedQuery[]> {
    maxQueries = Math.max(1, Math.min(4, Math.floor(maxQueries) || 1));
    if (!this.llmProvider) {
      return planQueriesDeterministically(question, freshness, maxQueries);
    }

    const curDate = currentDate || new Date().toISOString().split('T')[0];

    const systemPrompt = `You are a web-search query planner.
Your ONLY job is to produce search queries required to answer the user's question.
Do NOT answer the question.

Generate the smallest number of searches (between 1 and ${maxQueries}) necessary to gather independent, high-quality evidence.

Prefer:
- exact entity names
- important keywords
- official documentation searches when appropriate
- primary sources where possible
- benchmark names when relevant

Current Date: ${curDate}

Return valid JSON: {"queries": [{"query": "...", "purpose": "...", "freshness": "month"}]}`;

    const userPrompt = `User question: "${question}"`;

    try {
      const res = await this.llmProvider.generate({
        systemPrompt,
        userPrompt,
        temperature: 0.1,
        maxTokens: 400,
        responseSchema: {
          ...QUERY_PLAN_SCHEMA,
          properties: { queries: { ...QUERY_PLAN_SCHEMA.properties.queries, maxItems: maxQueries } },
        },
      });

      const match = res.text.match(/\{[\s\S]*\}/);
      if (match) {
        const parsed = JSON.parse(match[0]);
        if (Array.isArray(parsed.queries) && parsed.queries.length > 0) {
          const seen = new Set<string>();
          const queries = parsed.queries.filter((q: any) => typeof q?.query === 'string' && q.query.trim().length > 0 && q.query.length <= 500)
            .filter((q: any) => { const key = q.query.toLowerCase().trim(); if(seen.has(key)) return false; seen.add(key); return true; }).slice(0, maxQueries).map((q: any) => ({
            query: String(q.query || '').trim(),
            purpose: String(q.purpose || 'Information retrieval').trim(),
            freshness: (['day','week','month','year','any','realtime'].includes(q.freshness) ? q.freshness as FreshnessWindow : freshness),
          }));
          if (queries.length) return queries;
        }
      }
    } catch {
      // Fallback on parse/model failure
    }

    return [{ query: question, purpose: 'Original question after planner failure', freshness }];
  }
}
