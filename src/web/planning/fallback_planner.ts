import type { FreshnessWindow, PlannedQuery } from '../types';

const CURRENT = /\b(latest|current|newest|now|today|recent|upcoming)\b/i;
const STOP = new Set(['a', 'an', 'the', 'is', 'are', 'do', 'does', 'what', 'which', 'who', 'when', 'where', 'how', 'can', 'could', 'i', 'you', 'me', 'my', 'give', 'show', 'tell', 'find', 'get', 'about', 'for', 'of', 'in', 'on', 'at', 'to', 'from', 'between', 'please', 'changed', 'compare', 'comparison']);

/** Copy explicit comparison subjects; never invent names from a pronoun. */
export function comparisonSubjects(question: string): string[] {
  const candidates = [
    question.match(/\bbetween\s+(.+?)\s+(?:and|vs\.?|versus)\s+([^?;\n]+)/i),
    question.match(/\bfor\s+(.+?)\s+(?:and|vs\.?|versus)\s+([^?;\n]+)/i),
    question.match(/\bcompare\s+(.+?)\s+(?:and|vs\.?|versus)\s+([^?;\n]+)/i),
    question.match(/^(.+?)\s+(?:vs\.?|versus)\s+([^?;\n]+)/i),
    /\bbenchmarks?|scores?\b/i.test(question) ? question.match(/^(.+?)\s+and\s+([^?;\n]+)/i) : null,
  ];
  for (const match of candidates) {
    if (!match) continue;
    const names = match.slice(1).map(name => name
      .replace(/\b(?:for|on|using|with|benchmarks?|coding|scores?|results?|performance|comparison)\b.*$/i, '')
      .replace(/[.!?]+$/, '').trim());
    if (names.every(name => name.length >= 2 && name.length <= 80
      && !/\b(?:these|those|this|that|both|chart|create)\b/i.test(name))) return names;
  }
  return [];
}

export function localDateString(now = new Date()): string {
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
}

export function compactQuery(raw: string, needsYear: boolean, year: string): string {
  if (hasSearchSyntax(raw)) return raw.trim();
  const words = raw.replace(/[“”"'?!,:;()]/g, ' ').split(/\s+/).filter(Boolean)
    .filter(word => !STOP.has(word.toLowerCase()));
  const temporalIndex = words.findIndex(word => CURRENT.test(word));
  const temporal = words[temporalIndex];
  const body = words.filter(word => !CURRENT.test(word));
  if (temporal) body.splice(Math.min(Math.max(1, temporalIndex), body.length), 0, temporal);
  if (needsYear && !body.includes(year)) body.push(year);
  return (needsYear ? [...body.filter(word => word !== year).slice(0, 7), year] : body.slice(0, 8)).join(' ');
}

export function hasSearchSyntax(query: string): boolean {
  return /\b(?:site|filetype|intitle):|(?:^|\s)-\S+|"[^"]+"/.test(query);
}

export function planQueriesDeterministically(
  question: string,
  freshness: FreshnessWindow = 'any',
  maxQueries = 4,
  currentDate = localDateString(),
): PlannedQuery[] {
  const year = currentDate.slice(0, 4);
  const subjects = comparisonSubjects(question);
  // Explicit question boundaries keep later parts from being truncated out of
  // a single eight-word fallback query. Do not split names joined by "and".
  const parts = question.split(/(?:\?(?:\s+|$)|[\n;]+)/).map(part => part.trim()).filter(Boolean);
  if (parts.length > 1 && subjects.length !== 2) {
    return parts.flatMap(part => planQueriesDeterministically(part, freshness, maxQueries, currentDate))
      .filter((item, index, all) => all.findIndex(other => other.query.toLowerCase() === item.query.toLowerCase()) === index)
      .slice(0, Math.max(1, maxQueries));
  }
  const needsYear = CURRENT.test(question);
  let candidates: string[];
  if (subjects.length === 2) {
    const [first, second] = subjects;
    const topic = /coding/i.test(question) ? /knowledge|business|science|math/i.test(question)
      ? 'benchmarks' : 'coding benchmarks' : /benchmark/i.test(question) ? 'benchmarks' : 'official information';
    candidates = [`${first} ${topic}`, `${second} ${topic}`, `${first} vs ${second} ${topic}`];
  } else {
    candidates = [question];
    if (/\b(reference|list|recommend|recommendation)\b/i.test(question)) candidates = [`${question} official list`];
  }
  return candidates.map(query => ({
    query: compactQuery(query, needsYear, year),
    purpose: 'Find evidence for the user question',
    freshness,
  })).filter(item => Boolean(item.query))
    .filter((item, index, all) => all.findIndex(other => other.query.toLowerCase() === item.query.toLowerCase()) === index)
    .slice(0, Math.max(1, maxQueries));
}
