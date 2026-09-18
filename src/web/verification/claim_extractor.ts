/**
 * Claim Extractor: Decomposes a synthesized answer into externally verifiable atomic claims.
 */

export function extractAtomicClaims(answerText: string): string[] {
  // Strip markdown headers, bullet symbols, and citation tags
  const clean = answerText
    .replace(/^#+\s+.*$/gm, '')
    .replace(/\[S\d+(?:\s*,\s*S\d+)*\]/gi, '')
    .trim();

  // Split into sentences
  const rawSentences = clean
    .split(/(?<=[.?!])\s+|\n+/)
    .map((s) => s.replace(/^[•\-\*\d\.]+\s*/, '').trim())
    .filter((s) => s.length > 15);

  const claims: string[] = [];

  for (const sentence of rawSentences) {
    if (/^(based on current retrieved sources|I couldn't verify|Sources disagree:)/i.test(sentence) && !sentence.startsWith('Sources disagree:')) continue;
    // Ignore conversational filler (e.g. "Here is what I found", "Based on current data", etc.)
    if (
      /^(here is|according to|in summary|to summarize|overall|based on|as mentioned)/i.test(sentence) &&
      sentence.length < 35
    ) {
      continue;
    }
    claims.push(sentence);
  }

  return claims;
}
