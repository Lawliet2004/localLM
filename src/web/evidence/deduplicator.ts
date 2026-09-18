/**
 * Evidence Deduplicator: Clusters semantically similar facts from multiple sources
 * into consolidated claims with multi-source attribution.
 */

import type { ExtractedFact, EvidenceClaim } from '../types';
import { tokenJaccardSimilarity } from '../ranking/deduplicator';

export function deduplicateFacts(facts: ExtractedFact[]): EvidenceClaim[] {
  const claims: EvidenceClaim[] = [];
  let claimCounter = 1;

  for (const fact of facts) {
    if (!fact.statement || fact.statement.trim().length === 0) continue;

    let merged = false;
    for (const claim of claims) {
      const sim = tokenJaccardSimilarity(claim.claim, fact.statement);
      const numbers = (text: string) => (text.match(/\b\d+(?:\.\d+)*(?:k|m|b|t)?\b/gi) || []).map(n => n.toLowerCase()).sort().join('|');
      const negative = (text: string) => /\b(not|never|no|cannot|doesn't)\b/i.test(text);
      if (sim >= 0.70 && numbers(claim.claim) === numbers(fact.statement) && negative(claim.claim) === negative(fact.statement)) {
        // High semantic similarity: corroborate the existing claim
        if (!claim.supportingSources.includes(fact.sourceId)) {
          claim.supportingSources.push(fact.sourceId);
        }
        claim.confidence = Math.min(1.0, Math.max(claim.confidence, fact.confidence) + 0.05);
        merged = true;
        break;
      }
    }

    if (!merged) {
      claims.push({
        id: `CLM-${claimCounter++}`,
        claim: fact.statement,
        supportingSources: [fact.sourceId],
        status: 'supported',
        confidence: fact.confidence,
      });
    }
  }

  return claims;
}
