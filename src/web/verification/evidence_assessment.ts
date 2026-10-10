import type { EvidenceClaim } from '../types';

const NUMBER = /[-+]?\b\d+(?:[.,]\d+)*(?:k|m|b|t)?\b/gi;
const normalize = (text: string) => text.toLowerCase().replace(/[^a-z0-9#%$€£]+/g, ' ').trim().replace(/\s+/g, ' ');

function quantity(value: string): string {
  const ungrouped = /^[+-]?\d{1,3}(?:,\d{3})+(?:\.\d+)?[kmbt]?$/i.test(value) ? value.replace(/,/g, '') : value;
  const decimal = ungrouped.match(/^([+-]?)(\d+)(?:\.(\d+))?([kmbt]?)$/i);
  if (!decimal) return value.toLowerCase(); // Version identifiers stay literal.
  const integer = decimal[2].replace(/^0+(?=\d)/, '');
  const fraction = decimal[3]?.replace(/0+$/, '') ?? '';
  return `${decimal[1] === '-' && (integer !== '0' || fraction) ? '-' : ''}${integer}${fraction ? `.${fraction}` : ''}${decimal[4].toLowerCase()}`;
}

interface Assertion { key: string; quantities: string; negative: boolean }

/** Exact assertion shapes only: shared vocabulary alone does not establish disagreement. */
function assertions(text: string): Assertion[] {
  const benchmark = text.match(/^Benchmark (.+?): (.+?)\.(?:\s|$)/i);
  if (benchmark) {
    const conditions = normalize(text.slice(benchmark[0].length));
    return benchmark[2].split(';').flatMap(cell => {
      const score = cell.trim().match(/^(.+?)\s+([-+]?\d[\d.,]*(?:\s*%|\s+Elo|\s*\/\s*\d+(?:\.\d+)?)?)$/i);
      return score ? [{ key: `benchmark ${normalize(benchmark[1])} ${normalize(score[1])} ${normalize(score[2].replace(NUMBER, '#'))} ${conditions}`,
        quantities: score[2].replace(NUMBER, quantity).toLowerCase(), negative: false }] : [];
    });
  }
  return text.split(/\n+|(?<=[.!?])\s+(?=[A-Z])/).flatMap(sentence => {
    const canonical = sentence.toLowerCase().replace(/\b(is|was|does|did|has|have|could|would|should)n't\b/g, '$1 not')
      .replace(/\bcan't\b/g, 'can not').replace(/\bcannot\b/g, 'can not');
    const property = canonical.match(/\b(?:price|fee|score|context window|parameter count)\b/)
      ?? (/\b(?:current|latest|newest|stable)\b/.test(canonical) ? canonical.match(/\bversion\b/) : null);
    const verb = canonical.match(/\b(?:is|are|was|were|has|have|supports?|scored?|achieves?|reaches?|contains?|costs?|suppl(?:y|ies)|announced|released|can|does|do|did|will)\b/);
    const predicate = property && (!verb || property.index! < verb.index!) ? property : verb;
    if (!predicate || !predicate.index) return [];
    const subject = canonical.slice(0, predicate.index);
    const statement = canonical.slice(predicate.index);
    const negative = /\b(?:not|no|never)\b/.test(statement);
    return [{ key: `${normalize(subject)} ${normalize(statement.replace(/\b(?:not|no|never)\b/g, '').replace(NUMBER, '#'))}`,
      quantities: (statement.match(NUMBER) ?? []).map(quantity).join('|'), negative }];
  });
}

/** Preserve source variants and flag only quantity/polarity changes in the same assertion. */
export function detectConservativeConflicts(claims: EvidenceClaim[]): EvidenceClaim[] {
  const assessed = claims.map(claim => ({ ...claim, supportingSources: [...claim.supportingSources],
    conflictingSources: [...(claim.conflictingSources ?? [])], variants: [...(claim.variants ?? [])] }));
  const shapes = assessed.map(claim => assertions(claim.claim));
  for (let i = 0; i < assessed.length; i++) {
    for (let j = i + 1; j < assessed.length; j++) {
      const conflict = shapes[i].some(first => shapes[j].some(second => first.key === second.key
        && (first.negative !== second.negative
          || Boolean(first.quantities && second.quantities && first.quantities !== second.quantities))));
      if (!conflict) continue;
      for (const [claim, other] of [[assessed[i], assessed[j]], [assessed[j], assessed[i]]]) {
        claim.status = 'conflicting';
        claim.assessment = 'conflicting';
        claim.confidence = Math.min(claim.confidence, 0.25);
        claim.conflictingSources = [...new Set([...claim.conflictingSources, ...other.supportingSources])];
        claim.variants = [...new Set([...claim.variants, claim.claim, other.claim])];
      }
    }
  }
  return assessed;
}
