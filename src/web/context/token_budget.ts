/**
 * Token Budget Manager: Strictly bounds context token consumption, prioritizing
 * high-confidence, primary-source, and direct-answer evidence while never truncating mid-sentence.
 */

import type { EvidenceClaim, TokenStats } from '../types';
import type { WebSearchContextConfig } from '../config/schema';
import { defaultTokenCounter } from '../chunking/tokenizer';

export interface BudgetAllocation {
  claims: EvidenceClaim[];
  tokenStats: TokenStats;
  droppedCount: number;
}

export function allocateEvidenceBudget(
  claims: EvidenceClaim[],
  question: string,
  config: WebSearchContextConfig
): BudgetAllocation {
  const questionTokens = defaultTokenCounter.count(question);
  const systemBudget = config.systemBudget;
  const questionBudget = Math.max(questionTokens, config.questionBudget);
  const safetyMargin = config.safetyMargin;

  // Evidence budget is capped to ensure total does not exceed totalInputBudget
  const maxAllowedEvidenceTokens = Math.max(
    0,
    config.totalInputBudget - systemBudget - questionBudget - safetyMargin
  );
  const evidenceBudget = Math.min(config.evidenceTokenBudget, maxAllowedEvidenceTokens);

  // Prioritize claims:
  // 1. Corroborated claims (multiple sources)
  // 2. High confidence
  // 3. Conflicting claims (to let model explain disagreements)
  const sorted = [...claims].sort((a, b) => {
    const aMult = a.supportingSources.length > 1 ? 1 : 0;
    const bMult = b.supportingSources.length > 1 ? 1 : 0;
    if (aMult !== bMult) return bMult - aMult;
    return b.confidence - a.confidence;
  });

  const selectedClaims: EvidenceClaim[] = [];
  let currentTokens = 0;
  let droppedCount = 0;

  for (const claim of sorted) {
    const claimText = `- ${claim.claim} [${claim.supportingSources.join(', ')}]\n`;
    const claimTokens = defaultTokenCounter.count(claimText);

    if (currentTokens + claimTokens <= evidenceBudget) {
      selectedClaims.push(claim);
      currentTokens += claimTokens;
    } else {
      droppedCount++;
    }
  }

  const tokenStats: TokenStats = {
    systemBudget,
    questionBudget,
    evidenceBudget,
    safetyMargin,
    totalInputBudget: config.totalInputBudget,
    estimatedEvidenceTokens: currentTokens,
    finalPromptTokens: systemBudget + questionTokens + currentTokens,
  };

  return {
    claims: selectedClaims,
    tokenStats,
    droppedCount,
  };
}
