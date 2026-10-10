import type { PlannedQuery, FreshnessWindow, LLMProvider } from '../types';
import { compactQuery, comparisonSubjects, hasSearchSyntax, localDateString, planQueriesDeterministically } from './fallback_planner';

const CURRENT = /\b(latest|current|newest|now|today|recent|upcoming)\b/i;

export class QueryPlanner {
  constructor(private llmProvider?: LLMProvider) {}

  async planQueries(question: string, freshness: FreshnessWindow = 'any', maxQueries = 4, currentDate = localDateString()): Promise<PlannedQuery[]> {
    // The caller passes the mode's ResearchBudget.initialQueries; this only
    // guards against a nonsensical value, it does not impose its own cap.
    maxQueries = Math.max(1, Math.floor(maxQueries) || 1);
    const fallback = () => planQueriesDeterministically(question, freshness, maxQueries, currentDate);
    if (hasSearchSyntax(question)) return [{ query: question.trim(), purpose: 'Explicit search query', freshness }];
    if (!this.llmProvider) return fallback();
    const year = currentDate.slice(0, 4);
    const needsYear = CURRENT.test(question);
    try {
      const response = await this.llmProvider.generate({
        systemPrompt: `Turn the user's question into 1-${maxQueries} web search queries. Today is ${currentDate}. Output one query per line and nothing else. Use only the main keywords: 1-8 words per query, never a full question. Search each requested part separately; include its subject in every query. For comparisons, search each named item separately and then their comparison. For latest/current/newest/now, include ${year}. Do not answer the question.`,
        userPrompt: `Question: ${question}`,
        temperature: 0,
        maxTokens: 128,
        disableThinking: true,
      });
      const lines = response.text.replace(/<think>[\s\S]*?(?:<\/think>|$)/gi, '')
        .replace(/^\s*```(?:\w+)?\s*$/gm, '').trim().split(/\r?\n/)
        .map(line => line.trim().replace(/^(?:\d+[.)]|[-*•])\s+/, '').trim()).filter(Boolean);
      const anchors = question.toLowerCase().match(/[a-z0-9][a-z0-9.+-]*/g)?.filter(word => word.length >= 3 && !/^(what|which|where|when|who|how|can|could|the|you|compare|between|and|for|latest|current|newest|version|benchmarks|coding)$/.test(word)) ?? [];
      if (!lines.length || lines.length > maxQueries || lines.some(line => /[?{}<>]/.test(line) || line.split(/\s+/).length > 8 || !anchors.some(anchor => line.toLowerCase().includes(anchor)))) return fallback();
      const queries = lines.map(line => compactQuery(line, needsYear, year)).filter(Boolean)
        .filter((line, index, all) => all.findIndex(other => other.toLowerCase() === line.toLowerCase()) === index);
      if (!queries.length) return fallback();
      const subjects = comparisonSubjects(question);
      if (subjects.some(subject => !queries.some(query => query.toLowerCase().includes(subject.toLowerCase())))) return fallback();
      if (/\b(compare|versus|vs\.?|between)\b/i.test(question) && queries.length < Math.min(3, maxQueries)) return fallback();
      return queries.map(query => ({ query, purpose: 'Find evidence for the user question', freshness }));
    } catch {
      return fallback();
    }
  }
}
