/**
 * Claim Verifier: Audits generated answer claims against retrieved evidence
 * to detect hallucinations and report evidence grounding.
 */

import type { EvidenceClaim, VerificationReport, VerifiedClaim, ClaimStatus, LLMProvider } from '../types';
import { tokenJaccardSimilarity } from '../ranking/deduplicator';

export class ClaimVerifier {
  constructor(private llmProvider?: LLMProvider) {}

  /**
   * Deterministic claim verification based on lexical & entity alignment with evidence.
   */
  verifyClaimsDeterministic(
    claims: string[],
    evidence: EvidenceClaim[]
  ): VerificationReport {
    const verifiedClaims: VerifiedClaim[] = [];
    let supportedCount = 0;
    let unsupportedCount = 0;
    let conflictingCount = 0;

    for (const claim of claims) {
      let bestScore = 0;
      let matchingEvidence: EvidenceClaim | null = null;
      let bestNumbersMatch = false;

      const tokenize = (s: string) =>
        s
          .toLowerCase()
          .replace(/[^\w\s]/g, ' ')
          .split(/\s+/)
          .filter((t) => t.length > 1);

      const claimTokens = tokenize(claim);
      const claimNums: string[] = Array.from(claim.match(/\b\d+(?:\.\d+)?(?:k|m|b|t)?\b/gi) || []).map((n) => n.toLowerCase());

      for (const ev of evidence) {
        const evTokens = new Set(tokenize(ev.claim));
        const evNums: string[] = Array.from(ev.claim.match(/\b\d+(?:\.\d+)?(?:k|m|b|t)?\b/gi) || []).map((n) => n.toLowerCase());

        let matched = 0;
        for (const t of claimTokens) {
          if (evTokens.has(t)) matched++;
        }
        const coverage = claimTokens.length > 0 ? matched / claimTokens.length : 1.0;
        const jaccard = tokenJaccardSimilarity(claim, ev.claim);
        const combinedScore = Math.max(coverage, jaccard);

        const negative = (text: string) => /\b(not|never|no|cannot|doesn't|isn't|wasn't|unable)\b/i.test(text);
        const numsMatch = (claimNums.length === 0 || claimNums.every((n) => evNums.includes(n))) && negative(claim) === negative(ev.claim);

        if (combinedScore > bestScore) {
          bestScore = combinedScore;
          matchingEvidence = ev;
          bestNumbersMatch = numsMatch;
        }
      }

      let status: ClaimStatus = 'UNSUPPORTED';
      let sources: string[] = [];

      if (matchingEvidence) {
        if (matchingEvidence.status === 'conflicting') {
          status = 'CONFLICTING';
          sources = matchingEvidence.supportingSources;
          conflictingCount++;
        } else if (bestNumbersMatch && bestScore >= 0.60) {
          status = 'SUPPORTED';
          sources = matchingEvidence.supportingSources;
          supportedCount++;
        } else if (bestNumbersMatch && bestScore >= 0.40) {
          status = 'PARTIALLY_SUPPORTED';
          sources = matchingEvidence.supportingSources;
          unsupportedCount++;
        } else {
          status = 'UNSUPPORTED';
          unsupportedCount++;
        }
      } else {
        status = 'UNSUPPORTED';
        unsupportedCount++;
      }

      verifiedClaims.push({
        claim,
        status,
        sources,
      });
    }

    return {
      claims: verifiedClaims,
      allSupported: unsupportedCount === 0 && conflictingCount === 0,
      supportedCount,
      unsupportedCount,
      conflictingCount,
    };
  }

  async verifyClaims(
    claims: string[],
    evidence: EvidenceClaim[]
  ): Promise<VerificationReport> {
    if (claims.length === 0) {
      return {
        claims: [],
        allSupported: true,
        supportedCount: 0,
        unsupportedCount: 0,
        conflictingCount: 0,
      };
    }

    if (!this.llmProvider) {
      return this.verifyClaimsDeterministic(claims, evidence);
    }

    const evidenceList = evidence.map((e, idx) => `[E${idx + 1}] ${e.claim} (Sources: ${e.supportingSources.join(', ')})`).join('\n');
    const claimList = claims.map((c, idx) => `[C${idx + 1}] ${c}`).join('\n');

    const systemPrompt = `You are a claim verification system.
Determine whether each claim is directly supported by the supplied evidence.

Rules:
- SUPPORTED: Evidence clearly establishes the claim.
- PARTIALLY_SUPPORTED: Partially confirmed or minor details unverified.
- UNSUPPORTED: Evidence does not establish the claim.
- CONFLICTING: Supplied evidence disagrees.
Output valid JSON: {"claims": [{"claim": "...", "status": "SUPPORTED|UNSUPPORTED|CONFLICTING", "sources": ["S1"]}]}`;

    const userPrompt = `Evidence:\n${evidenceList}\n\nClaims to verify:\n${claimList}`;

    try {
      const res = await this.llmProvider.generate({
        systemPrompt,
        userPrompt,
        temperature: 0.1,
        maxTokens: 600,
      });

      const jsonMatch = res.text.match(/\{[\s\S]*\}/);
      if (jsonMatch) {
        const parsed = JSON.parse(jsonMatch[0]);
        if (Array.isArray(parsed.claims) && parsed.claims.length === claims.length && parsed.claims.every((item: unknown, index: number) => {
          if (!item || typeof item !== 'object') return false;
          const row = item as VerifiedClaim;
          return row.claim === claims[index] && ['SUPPORTED','PARTIALLY_SUPPORTED','UNSUPPORTED','CONFLICTING'].includes(row.status)
            && Array.isArray(row.sources) && row.sources.every(id => evidence.some(e => e.supportingSources.includes(id)))
            && (row.status !== 'SUPPORTED' || row.sources.length > 0);
        })) {
          let sup = 0;
          let unsup = 0;
          let conf = 0;
          const vClaims: VerifiedClaim[] = parsed.claims.map((item: any) => {
            const st: ClaimStatus = item.status || 'UNSUPPORTED';
            if (st === 'SUPPORTED') sup++;
            else if (st === 'CONFLICTING') conf++;
            else unsup++;
            return {
              claim: item.claim,
              status: st,
              sources: Array.isArray(item.sources) ? item.sources : [],
            };
          });

          return {
            claims: vClaims,
            allSupported: unsup === 0 && conf === 0,
            supportedCount: sup,
            unsupportedCount: unsup,
            conflictingCount: conf,
          };
        }
      }
    } catch {
      // Fallback to deterministic verification
    }

    return this.verifyClaimsDeterministic(claims, evidence);
  }
}
