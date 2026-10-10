/**
 * Claim Extractor: Decomposes a synthesized answer into externally verifiable atomic claims.
 */

export function extractAtomicClaims(answerText: string): string[] {
  // Reference lists and citation formatting are provenance, not new claims.
  let references = false;
  const body = answerText.split('\n').filter((line) => {
    if (/^\s*(?:#{1,6}\s*)?(?:supporting sources|sources consulted|sources|references|citations)\s*:?\s*$/i.test(line)) {
      references = true;
      return false;
    }
    if (!references || !line.trim()) return true;
    if (/https?:\/\//i.test(line)) return false;
    // A new heading or prose paragraph resumes the substantive answer.
    references = false;
    return true;
  }).join('\n');
  const clean = body
    .replace(/^#+\s+.*$/gm, '')
    .replace(/^\s*\[[^\]\n]+\]:\s*https?:\/\/.*$/gm, '')
    .replace(/\[([^\]\n]+)\]\(\s*<?https?:\/\/(?:\([^)]*\)|[^)\s])*?>?\s*\)/gi,
      (link: string, label: string, offset: number, text: string) => {
        const before = text.slice(text.lastIndexOf('\n', offset) + 1, offset);
        const after = text.slice(offset + link.length).split('\n', 1)[0].trim();
        const citation = /^(?:sources?|s\d+|references?|citations?|release notes|official (?:source|documentation|docs?)|docs?|documentation|read more|link|here)$/i.test(label.trim());
        const trailingReference = /[.!?]\s*$/.test(before) && /^(?:[.,;:]|\[|$)/.test(after);
        return citation || trailingReference ? '' : label;
      })
    .replace(/\[S\d+(?:\s*,\s*S\d+)*\]/gi, '')
    .replace(/<?https?:\/\/[^\s<>()[\]]+>?/gi, '')
    .replace(/[ \t]+([.,;:?!])/g, '$1')
    .replace(/[ \t]{2,}/g, ' ')
    .trim();

  // Split into sentences
  const rawSentences = clean
    .split(/(?<=[.?!])\s+|\n+/)
    .map((s) => s.replace(/^[•\-*\d.]+\s*/, '').trim())
    .filter((s) => s.length > 15);

  const claims: string[] = [];

  for (let sentence of rawSentences) {
    sentence = sentence.replace(/^based on current retrieved sources\b[:,]?\s*/i, '');
    if (/^I (?:couldn't|could not|cannot|can't) verify\b/i.test(sentence)) {
      const assertion = sentence.match(/[,;]\s*(?:but|however|yet)\b[:,]?\s*(.+)$/i);
      if (!assertion) continue;
      sentence = assertion[1];
    }
    if (sentence.length <= 15) continue;
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
