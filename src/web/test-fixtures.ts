import { vi } from 'vitest';
import { HttpFetcher } from './fetch/http_fetcher';
import { WebSearchEngine } from './index';
import type { FreshnessWindow, SearchProvider, SearchRequest, SearchResult, VerticalType } from './types';

/** Explicit HTTP boundary fixture. Production providers never return fixture data. */
export function installWebFixtures() {
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('geocoding-api.open-meteo')) return Response.json({results:[{name:'Ranaghat',admin1:'West Bengal',country:'India',latitude:23.18,longitude:88.58}]});
    if (url.includes('api.open-meteo')) return Response.json({timezone:'Asia/Kolkata', current:{time:'2026-09-16T12:00',temperature_2m:31.2,apparent_temperature:36,relative_humidity_2m:78,wind_speed_10m:12},daily:{time:['2026-09-16'],temperature_2m_min:[26],temperature_2m_max:[32],precipitation_probability_max:[65]}});
    return new Response('Unavailable', {status:404});
  }));
  vi.spyOn(HttpFetcher.prototype, 'fetch').mockImplementation(async (url) => {
    const res = await fetch(url);
    return {url,finalUrl:url,success:res.ok,status:res.status,body:await res.text(),durationMs:0};
  });
}

export class MockSearchProvider implements SearchProvider {
  constructor(private resultsMap: Record<string, SearchResult[]> = {}) {}

  async search(request: SearchRequest): Promise<SearchResult[]> {
    const qLower = request.query.toLowerCase();
    for (const [key, results] of Object.entries(this.resultsMap)) {
      if (qLower.includes(key.toLowerCase()) || key.toLowerCase().includes(qLower)) {
        return results;
      }
    }
    return [];
  }
}

export interface EvalQuestion {
  id: string;
  category: string;
  question: string;
  expectedVertical: VerticalType;
  expectedFreshness: FreshnessWindow;
  shouldHaveCitations: boolean;
}

export interface EvalScorecard {
  totalQuestions: number;
  evaluated: number;
  routingAccuracy: number;
  unsupportedClaimCount: number;
  citationCorrectnessRate: number;
  averageContextTokens: number;
  averageLatencyMs: number;
  categoryBreakdown: Record<string, { count: number; passed: number }>;
}

/**
 * Test-only evaluation harness: runs questions through the engine and scores
 * routing, citation integrity, and context efficiency.
 */
export class WebSearchEvaluator {
  constructor(private engine: WebSearchEngine) {}

  async evaluateQuestions(questions: EvalQuestion[]): Promise<EvalScorecard> {
    let routingCorrect = 0;
    let totalUnsupported = 0;
    let validCitations = 0;
    let totalCitations = 0;
    let totalContextTokens = 0;
    let totalLatency = 0;

    const categoryBreakdown: Record<string, { count: number; passed: number }> = {};

    for (const q of questions) {
      const cat = q.category;
      if (!categoryBreakdown[cat]) {
        categoryBreakdown[cat] = { count: 0, passed: 0 };
      }
      categoryBreakdown[cat].count++;

      const session = await this.engine.research(q.question);

      const routePassed = session.route.vertical === q.expectedVertical;
      if (routePassed) routingCorrect++;

      const citations = session.answer?.match(/\[S\d+\]/g) || [];
      totalCitations += citations.length;
      for (const cit of citations) {
        const id = cit.replace(/[[\]]/g, '');
        if (session.sources[id]) validCitations++;
      }

      const missingRequired = q.shouldHaveCitations && citations.length === 0;

      if (session.verification) {
        totalUnsupported += session.verification.unsupportedCount;
      }

      totalContextTokens += session.tokenUsage.finalPromptTokens;
      totalLatency += session.trace.totalLatencyMs;

      const answerEmpty = !session.answer || session.answer.trim().length === 0;
      const unsupported = session.verification?.unsupportedCount ?? 1;
      const requirements = (session as { requirements?: Array<{ status: string }> }).requirements || [];
      const unresolved = requirements.filter((r) => r.status !== 'answered').length;
      if (!answerEmpty && !missingRequired && routePassed && unsupported === 0 && unresolved === 0) {
        categoryBreakdown[cat].passed++;
      }
    }

    const count = questions.length;
    return {
      totalQuestions: count,
      evaluated: count,
      routingAccuracy: count > 0 ? Number((routingCorrect / count).toFixed(2)) : 1,
      unsupportedClaimCount: totalUnsupported,
      citationCorrectnessRate:
        totalCitations > 0 ? Number((validCitations / totalCitations).toFixed(2)) : 0,
      averageContextTokens: count > 0 ? Math.round(totalContextTokens / count) : 0,
      averageLatencyMs: count > 0 ? Math.round(totalLatency / count) : 0,
      categoryBreakdown,
    };
  }
}
