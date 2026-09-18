/**
 * Structured Research Result: answer, claims, evidence references, source
 * metadata, unresolved requirements, limitations, and completion status.
 *
 * Render a readable Markdown report by default; emit validated JSON when
 * requested. Logs, diagnostics, and raw tool results stay separate from the
 * final answer. Claims track entity/subject, dates, numbers/units, and
 * qualifications against the original source passages — a source ID existing
 * in the registry never establishes citation correctness by itself.
 */

import type { EvidenceClaim, SourceRegistry, VerificationReport } from '../types';
import { validateAndCleanCitations } from '../generation/citation_renderer';
import { extractAtomicClaims } from './claim_extractor';
import { ClaimVerifier } from './claim_verifier';

export type CompletionStatus = 'complete' | 'partial' | 'unresolved' | 'blocked';

export interface StructuredClaim {
  text: string;
  status: string;
  sources: string[];
  supported: boolean;
  numbers: string[];
  dates: string[];
}

export interface StructuredResearchResult {
  schemaVersion: 'locallm.research-result/1';
  answer: string;
  claims: StructuredClaim[];
  evidenceRefs: string[];
  sources: Array<{ id: string; title: string; url: string; domain: string; publishedAt?: string }>;
  unresolvedRequirements: Array<{ text: string; reason: string }>;
  limitations: string[];
  completionStatus: CompletionStatus;
}

export function extractNumbers(text: string): string[] {
  return Array.from(text.match(/\b\d+(?:\.\d+)?(?:\s?(?:k|m|b|t|ms|s|gb|mb|%))?\b/gi) || []).map((n) => n.toLowerCase());
}

export function extractDates(text: string): string[] {
  return Array.from(
    text.match(/\b(19|20)\d{2}(?:-\d{2}(?:-\d{2})?)?\b|\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\s+\d{1,2},?\s+\d{4}\b/gi) || [],
  );
}

export function buildStructuredResult(
  answerMarkdown: string,
  evidence: EvidenceClaim[],
  sources: SourceRegistry,
  verification: VerificationReport | undefined,
  unresolved: Array<{ text: string; reason: string }>,
  limitations: string[],
): StructuredResearchResult {
  const verifier = new ClaimVerifier();
  const atomic = extractAtomicClaims(answerMarkdown);
  const report = verification || verifier.verifyClaimsDeterministic(atomic, evidence);
  const validation = validateAndCleanCitations(answerMarkdown, sources);

  const claims: StructuredClaim[] = report.claims.map((claim) => {
    const supported = claim.status === 'SUPPORTED';
    // Citation correctness: every cited source must exist AND actually
    // support the claim (checked against that source's own evidence).
    const citedValid = claim.sources.every((id) => {
      if (!sources[id]) return false;
      const own = evidence.filter((c) => c.supportingSources.includes(id));
      if (own.length === 0) return false;
      return verifier.verifyClaimsDeterministic([claim.claim], own).supportedCount > 0;
    });
    return {
      text: claim.claim,
      status: claim.status,
      sources: claim.sources,
      supported: supported && citedValid,
      numbers: extractNumbers(claim.claim),
      dates: extractDates(claim.claim),
    };
  });

  const evidenceRefs = [...new Set(evidence.flatMap((c) => c.supportingSources))].filter((id) => sources[id]);
  const sourceList = evidenceRefs.map((id) => ({
    id,
    title: sources[id].title,
    url: sources[id].url,
    domain: sources[id].domain,
    publishedAt: sources[id].publishedAt,
  }));

  const allSupported = claims.length > 0 && claims.every((c) => c.supported);
  const completionStatus: CompletionStatus =
    unresolved.length > 0 && claims.length === 0 ? 'blocked'
    : unresolved.length > 0 || !allSupported ? 'partial'
    : 'complete';

  return {
    schemaVersion: 'locallm.research-result/1',
    answer: validation.cleanedText,
    claims,
    evidenceRefs,
    sources: sourceList,
    unresolvedRequirements: unresolved,
    limitations,
    completionStatus,
  };
}

export function renderReport(result: StructuredResearchResult): string {
  const lines: string[] = [result.answer, '\n---\n'];
  lines.push(`**Status:** ${result.completionStatus} · **Claims:** ${result.claims.filter((c) => c.supported).length}/${result.claims.length} supported`);
  if (result.unresolvedRequirements.length > 0) {
    lines.push('\n**Unresolved:**');
    for (const req of result.unresolvedRequirements) lines.push(`- ${req.text} — ${req.reason}`);
  }
  if (result.limitations.length > 0) {
    lines.push('\n**Limitations:**');
    for (const limitation of result.limitations) lines.push(`- ${limitation}`);
  }
  const registry: SourceRegistry = Object.fromEntries(result.sources.map((s) => [s.id, s as never]));
  void registry;
  return lines.join('\n');
}
