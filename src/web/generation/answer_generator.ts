/**
 * Answer Generator: Generates grounded answers using local LLM inference or deterministic
 * synthesis, validates citations, and formats clickable citations.
 */

import type { LLMProvider, SourceRegistry, EvidenceClaim } from '../types';
import type { GroundedContextPackage } from '../context/context_builder';
import { validateAndCleanCitations, renderMarkdownCitations, renderSourcesSection } from './citation_renderer';
import { ClaimVerifier } from '../verification/claim_verifier';
import { extractAtomicClaims } from '../verification/claim_extractor';

export interface AnswerResult {
  text: string;
  markdownWithCitations: string;
  citedSourceIds: string[];
  invalidSourceIds: string[];
  method: 'llm' | 'deterministic_synthesis';
}

export class AnswerGenerator {
  constructor(private llmProvider?: LLMProvider) {}

  /**
   * Deterministic synthesis if LLM is unavailable or fails.
   */
  deterministicSynthesize(
    question: string,
    claims: EvidenceClaim[],
    sources: SourceRegistry
  ): AnswerResult {
    if (claims.length === 0) {
      return {
        text: "I couldn't verify this information from the retrieved sources.",
        markdownWithCitations: "I couldn't verify this information from the retrieved sources.",
        citedSourceIds: [],
        invalidSourceIds: [],
        method: 'deterministic_synthesis',
      };
    }

    const lines: string[] = [];
    lines.push(`Based on current retrieved sources for "${question}":\n`);

    const citedSet = new Set<string>();
    for (const claim of claims) {
      const sourceTags = claim.supportingSources.map((s) => `[${s}]`).join(' ');
      lines.push(`• ${claim.status === 'conflicting' ? 'Sources disagree: ' : ''}${claim.claim} ${sourceTags}`);
      claim.supportingSources.forEach((s) => citedSet.add(s));
    }

    const rawText = lines.join('\n');
    const citedList = Array.from(citedSet);
    const mdWithCitations = renderMarkdownCitations(rawText, sources) + renderSourcesSection(citedList, sources);

    return {
      text: rawText,
      markdownWithCitations: mdWithCitations,
      citedSourceIds: citedList,
      invalidSourceIds: [],
      method: 'deterministic_synthesis',
    };
  }

  async generateAnswer(
    context: GroundedContextPackage,
    sources: SourceRegistry,
    claims: EvidenceClaim[],
    question: string
  ): Promise<AnswerResult> {
    if (!this.llmProvider || (claims.length === 0 && context.systemPrompt.includes('grounded answer'))) {
      return this.deterministicSynthesize(question, claims, sources);
    }

    try {
      const response = await this.llmProvider.generate({
        systemPrompt: context.systemPrompt,
        userPrompt: context.userPrompt,
        temperature: 0.2,
        maxTokens: 1000,
      });

      const rawAnswer = response.text.trim();
      if (!rawAnswer) {
        return this.deterministicSynthesize(question, claims, sources);
      }
      if (claims.length > 0) {
        // Check support against the cited subset, not an unrelated source elsewhere
        // in the context. Reject model-written URLs; the registry owns all links.
        const auditor = new ClaimVerifier();
        const paragraphs = rawAnswer.split(/\n+/).filter(p => p.trim() && !/^#+\s/.test(p));
        const invalid = /https?:\/\//i.test(rawAnswer) || paragraphs.some(paragraph => {
          const statements = extractAtomicClaims(paragraph);
          if (!statements.length || /couldn't verify|cannot verify|insufficient evidence/i.test(paragraph)) return false;
          const ids: string[] = paragraph.match(/S\d+/g) || [];
          if (!ids.length) return true;
          const citedEvidence = claims.filter(c => c.supportingSources.some(id => ids.includes(id)));
          return !auditor.verifyClaimsDeterministic(statements, citedEvidence).allSupported
            || ids.some(id => !sources[id] || !claims.some(c => c.supportingSources.includes(id) && auditor.verifyClaimsDeterministic(statements, [c]).supportedCount > 0));
        });
        if (invalid) return this.deterministicSynthesize(question, claims, sources);
      }

      // Validate & clean citations
      const validation = validateAndCleanCitations(rawAnswer, sources);
      const markdown = renderMarkdownCitations(validation.cleanedText, sources) +
        renderSourcesSection(validation.citedSourceIds, sources);

      return {
        text: validation.cleanedText,
        markdownWithCitations: markdown,
        citedSourceIds: validation.citedSourceIds,
        invalidSourceIds: validation.invalidSourceIds,
        method: 'llm',
      };
    } catch {
      // Fallback on generation failure
      return this.deterministicSynthesize(question, claims, sources);
    }
  }
}
