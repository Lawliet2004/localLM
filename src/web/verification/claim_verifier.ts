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

      const passage = matchingEvidence?.claim ?? '';
      const exact = matchingEvidence ? excerptContains(claim, passage) : false;
      // Same numbers and polarity are not enough when the unit changed or the
      // claim adds current wording the passage does not contain.
      const grounded = exact
        && bestNumbersMatch
        && !quantitiesConflict(claim, passage)
        && !addsCurrentWording(claim, passage);
      const assessment = !matchingEvidence
        ? 'unresolved'
        : matchingEvidence.status === 'conflicting'
          ? 'conflicting'
          : grounded
            ? 'exact_excerpt'
            : bestScore >= 0.4
              ? 'lexical_match'
              : 'unresolved';
      if (matchingEvidence) {
        if (matchingEvidence.status === 'conflicting' || assessment === 'conflicting') {
          status = 'CONFLICTING';
          sources = matchingEvidence.supportingSources;
          conflictingCount++;
        } else if (grounded) {
          // Lexical overlap is not enough. Only an exact excerpt certifies the claim.
          status = 'SUPPORTED';
          sources = matchingEvidence.supportingSources;
          supportedCount++;
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
        assessment,
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
              assessment: 'model_assessed' as const,
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

function normalizeText(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^\w\s.%$€]/g, ' ')
    .replace(/(?<!\d)\.(?!\d)/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

const TRAILING_UNIT = /^(?:[kmbt]|kilometers?|km|miles?|mi|dollars?|usd|cents?|euros?|gb|mb|kb|tb|percent|watts?)(?!\w)/;
const CURRENT_MARKERS = ['as of now', 'right now', 'currently', 'nowadays', 'presently', 'current', 'today', 'latest'];

/** The quoted text has to occur in the passage. Overlap is not enough. */
export function excerptContains(quote: string, passage: string): boolean {
  const quoted = normalizeText(quote);
  const source = normalizeText(passage);
  if (!quoted) return false;
  let from = 0;
  while (from < source.length) {
    const at = source.indexOf(quoted, from);
    if (at < 0) return false;
    const end = at + quoted.length;
    const startsToken = at === 0 || source[at - 1] === ' ';
    const endsToken = end === source.length || source[end] === ' ';
    if (startsToken && endsToken && !TRAILING_UNIT.test(source.slice(end).trimStart())) return true;
    from = at + 1;
  }
  return false;
}

export function citationSupportsQuote(quote: string, passage: string): boolean {
  return excerptContains(quote, passage);
}

/** A URL the model wrote is not a retrieved source. */
export function trustedSourceUrl(url: string, retrievedUrls: string[]): boolean {
  return retrievedUrls.includes(url);
}

export function arithmeticRecord(sourceValues: number[], computed: number) {
  return { sourceValues: sourceValues.slice(), computed, kind: 'local_calculation' as const };
}

function canonicalUnit(symbol: string, raw: string): string | null {
  if (symbol === '$' || /^(?:dollars?|usd)$/i.test(raw)) return 'dollar';
  if (symbol === '€' || /^euros?$/i.test(raw)) return 'euro';
  if (raw === '%' || /^percent$/i.test(raw)) return 'percent';
  if (/^(?:kilometers?|km)$/i.test(raw)) return 'km';
  if (/^(?:miles?|mi)$/i.test(raw)) return 'mile';
  if (/^cents?$/i.test(raw)) return 'cent';
  if (/^watts?$/i.test(raw)) return 'watt';
  if (/^(?:gb|mb|kb|tb)$/i.test(raw)) return raw.toLowerCase();
  return null;
}

function extractQuantities(text: string): Array<{ value: string; unit: string | null }> {
  const found: Array<{ value: string; unit: string | null }> = [];
  const quantity = /(?<sym>[$€])?\s*\b(?<num>\d+(?:\.\d+)?)(?<suf>[kmbt])?\b(?:\s*(?<unit>kilometers?|km|miles?|mi|dollars?|usd|cents?|euros?|gb|mb|kb|tb|percent|%|watts?)(?!\w))?/gi;
  for (const match of text.matchAll(quantity)) {
    const num = match.groups?.num;
    if (!num) continue;
    const suffix = (match.groups?.suf ?? '').toLowerCase();
    found.push({
      value: `${num}${suffix}`.toLowerCase(),
      unit: canonicalUnit(match.groups?.sym ?? '', match.groups?.unit ?? ''),
    });
  }
  return found;
}

/** Same magnitude with a different or dropped unit is not the same quantity. */
function quantitiesConflict(claim: string, passage: string): boolean {
  const passageQuantities = extractQuantities(passage);
  for (const quantity of extractQuantities(claim)) {
    const sameValue = passageQuantities.filter((item) => item.value === quantity.value);
    if (quantity.unit) {
      if (!sameValue.some((item) => item.unit === quantity.unit)) return true;
    } else if (sameValue.length > 0 && sameValue.every((item) => item.unit)) {
      return true;
    }
  }
  return false;
}

function hasMarker(text: string, marker: string): boolean {
  const escaped = marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(?:^|\\s)${escaped}(?=\\s|$)`).test(text);
}

/** "Currently" / "latest" in the claim is not supported by an older passage that lacks it. */
function addsCurrentWording(claim: string, passage: string): boolean {
  const claimed = normalizeText(claim);
  const source = normalizeText(passage);
  return CURRENT_MARKERS.some((marker) => hasMarker(claimed, marker) && !hasMarker(source, marker));
}
