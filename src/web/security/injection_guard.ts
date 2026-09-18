/**
 * Injection Guard: Treats all retrieved web content as UNTRUSTED evidence data.
 * Web text must NEVER be interpreted as system instructions or override prompt instructions.
 */

export interface SanitizedEvidenceContent {
  sanitizedText: string;
  hasSuspiciousDirectives: boolean;
  neutralizedDirectives: string[];
}

const SUSPICIOUS_DIRECTIVE_PATTERNS: RegExp[] = [
  /ignore\s+(all\s+)?(previous|prior|above)\s+instructions/gi,
  /disregard\s+(all\s+)?(previous|prior|above)\s+instructions/gi,
  /you\s+are\s+now\s+(a|an|in)\s+/gi,
  /system\s+message\s*:/gi,
  /system\s+prompt\s*:/gi,
  /output\s+the\s+following\s+exact/gi,
  /send\s+(the\s+)?(data|ssh|key|credentials|password)/gi,
  /execute\s+(the\s+following\s+)?(command|script|code)/gi,
  /reveal\s+(the\s+)?(system|initial)\s+prompt/gi,
  /<!--\s*(system|instruction|prompt)[\s\S]*?-->/gi,
];

/**
 * Neutralizes prompt injection patterns by escaping/marking them clearly
 * as unexecutable quoted external claims across all occurrences.
 */
export function sanitizeWebEvidence(rawText: string): SanitizedEvidenceContent {
  const neutralizedDirectives: string[] = [];
  let sanitizedText = rawText;

  for (const pattern of SUSPICIOUS_DIRECTIVE_PATTERNS) {
    // Reset lastIndex for global regular expression
    pattern.lastIndex = 0;
    const matches = Array.from(sanitizedText.matchAll(pattern));
    for (const m of matches) {
      neutralizedDirectives.push(m[0]);
    }
    if (matches.length > 0) {
      pattern.lastIndex = 0;
      sanitizedText = sanitizedText.replace(pattern, '[UNTRUSTED_DIRECTIVE_NEUTRALIZED]');
    }
  }

  return {
    sanitizedText,
    hasSuspiciousDirectives: neutralizedDirectives.length > 0,
    neutralizedDirectives,
  };
}

/**
 * Wraps evidence in security containment tags for LLM ingestion.
 */
export function wrapUntrustedEvidence(sourceId: string, content: string): string {
  const { sanitizedText } = sanitizeWebEvidence(content);
  return `<!-- BEGIN UNTRUSTED EVIDENCE [${sourceId}] -->\n${sanitizedText}\n<!-- END UNTRUSTED EVIDENCE [${sourceId}] -->`;
}
