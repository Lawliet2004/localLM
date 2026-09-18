/**
 * Research Trace: Captures quantitative operational metrics and latencies across each search turn.
 */

import type { ResearchTrace } from '../types';

export class TraceCollector {
  private trace: ResearchTrace;
  private stageTimers = new Map<string, number>();

  constructor() {
    this.trace = {
      queriesGenerated: 0,
      searchResults: 0,
      uniqueResults: 0,
      pagesSelected: 0,
      pagesFetched: 0,
      fetchFailures: 0,
      extractedTokens: 0,
      chunksCreated: 0,
      chunksAfterRetrieval: 0,
      chunksAfterRerank: 0,
      evidenceClaims: 0,
      finalEvidenceTokens: 0,
      finalPromptTokens: 0,
      verifiedClaims: 0,
      unsupportedClaims: 0,
      conflictingClaims: 0,
      providerUsed: 'unknown',
      fallbackUsed: false,
      fallbackReason: '',
      latencies: {},
      totalLatencyMs: 0,
    };
  }

  startTimer(stage: string) {
    this.stageTimers.set(stage, Date.now());
  }

  endTimer(stage: string) {
    const start = this.stageTimers.get(stage);
    if (start) {
      this.trace.latencies[stage] = Date.now() - start;
      this.stageTimers.delete(stage);
    }
  }

  update(patch: Partial<ResearchTrace>) {
    Object.assign(this.trace, patch);
  }

  getTrace(): ResearchTrace {
    return { ...this.trace };
  }
}
