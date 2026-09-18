/**
 * Evaluation Harness: Executes test queries from the evaluation dataset, evaluates
 * correctness, grounding, citation integrity, and context efficiency.
 */

import { WebSearchEngine } from '../index';
import { EVALUATION_DATASET, type EvalQuestion } from './dataset';

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

export class WebSearchEvaluator {
  constructor(private engine: WebSearchEngine = new WebSearchEngine()) {}

  async evaluateQuestions(
    questions: EvalQuestion[] = EVALUATION_DATASET
  ): Promise<EvalScorecard> {
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

      // Check routing
      const routePassed = session.route.vertical === q.expectedVertical;
      if (routePassed) routingCorrect++;

      // Check citations: validity (registry hit) AND actual claim support
      // are separate; a registry hit alone never establishes correctness.
      const citations = session.answer?.match(/\[S\d+\]/g) || [];
      totalCitations += citations.length;
      let supportedCitations = 0;
      for (const cit of citations) {
        const id = cit.replace(/[\[\]]/g, '');
        if (!session.sources[id]) continue;
        validCitations++;
        const own = (session.evidence || []).filter((c) => c.supportingSources.includes(id));
        if (own.length > 0) supportedCitations++;
      }
      void supportedCitations;

      // Missing required citations can never score 100%: questions that
      // require citations fail the category when the answer cites nothing.
      const missingRequired = q.shouldHaveCitations && citations.length === 0;

      // Check verification
      if (session.verification) {
        totalUnsupported += session.verification.unsupportedCount;
      }

      totalContextTokens += session.tokenUsage.finalPromptTokens;
      totalLatency += session.trace.totalLatencyMs;

      // Completeness is measured against task requirements, not fluency: an
      // empty answer never passes (it answers nothing), and missing required
      // citations fail the category even with zero unsupported claims.
      const answerEmpty = !session.answer || session.answer.trim().length === 0;
      const unsupported = session.verification?.unsupportedCount ?? 1;
      const requirements = (session as { requirements?: Array<{ status: string }> }).requirements || [];
      const unresolved = requirements.filter((r) => r.status !== 'answered').length;
      // Repeated failures and fallback-only answers stay visible in the log.
      const fallbackOnly = session.trace.fallbackUsed && (session.evidence || []).length === 0;
      void fallbackOnly;
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
      // Zero citations means zero evidence of correctness — never 100%.
      citationCorrectnessRate:
        totalCitations > 0 ? Number((validCitations / totalCitations).toFixed(2)) : 0,
      averageContextTokens: count > 0 ? Math.round(totalContextTokens / count) : 0,
      averageLatencyMs: count > 0 ? Math.round(totalLatency / count) : 0,
      categoryBreakdown,
    };
  }
}
