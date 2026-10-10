import { comparisonSubjects } from '../planning/fallback_planner';
import { sanitizeWebEvidence } from '../security/injection_guard';

export interface BenchmarkRow {
  name: string;
  scores: Array<{ model: string; value: string }>;
  note?: string;
}

const normalizeLabel = (text: string) => text.normalize('NFKC').replace(/[‐‑‒–—−]/g, '-').replace(/\s+/g, ' ').trim();
const modelKey = (text: string) => normalizeLabel(text).replace(/^(?:OpenAI|Anthropic|Claude|Google|Meta)\s+/i, '').toLowerCase();

export function requestedBenchmarkCount(question: string, defaultCount = 3): number {
  const count = question.match(/\b(?:top|first|all)\s+(\d+)\s+benchmarks?\b/i)
    ?? question.match(/\b(\d+)\s+(?:(?:coding|and|knowledge|work)\s+)*benchmarks?\b/i);
  return count ? Math.max(1, Math.min(10, Number(count[1]))) : defaultCount;
}

export function mentionsName(text: string, name: string): boolean {
  const escaped = normalizeLabel(name).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(?:^|[^\\w.])${escaped}(?![\\w.])`, 'i').test(normalizeLabel(text));
}

export function benchmarkInScope(name: string, question: string): boolean {
  const codingOnly = /\b(coding|code|software|programming)\b/i.test(question)
    && !/\b(knowledge|business|science|scientific|math|reasoning)\b/i.test(question);
  return !codingOnly || /coding|code|software|programming|swe|terminal-bench|aider/i.test(name)
    && !/scientific|science|business|knowledge|math/i.test(name);
}

/** Keep original cells and exact model columns, including partial one-model rows. */
export function extractBenchmarkRows(text: string, question: string): BenchmarkRow[] {
  if (!/\b(benchmarks?|scores?|performance)\b/i.test(question)) return [];
  const clean = (value: string) => sanitizeWebEvidence(value.replace(/<[^>]*>/g, '')
    .replace(/\*\*/g, '').replace(/\[([^\]]+)\]\([^)]+\)/g, '$1').trim()).sanitizedText;
  const cells = (line: string) => line.split('|').slice(1, -1).map(clean);
  const lines = text.split(/\r?\n/).map(line => line.trim());
  const subjects = comparisonSubjects(question);
  const modelIdentity = (name: string) => modelKey(name).replace(/\s*\((?:x?high|max|medium|low)(?: effort)?\)$/, '').trim();
  const wanted = (column: string) => subjects.length
    ? subjects.some(subject => modelIdentity(column) === modelIdentity(subject)) : mentionsName(question, column);
  const rows: BenchmarkRow[] = [];
  let columns: Array<{ index: number; model: string }> = [];
  let noteColumns: number[] = [];
  for (const line of lines) {
    if (!line) continue;
    if (!line.startsWith('|')) { columns = []; continue; }
    if (/^\|\s*:?-{2,}/.test(line)) continue;
    const values = cells(line);
    const named = values.flatMap((column, index) => index > 0 && column.length > 2 && column.length <= 100
      && wanted(column)
      ? [{ index, model: column }] : []);
    if (named.length && (!columns.length || /^(?:benchmarks?)?$/i.test(values[0]))) {
      columns = named;
      noteColumns = values.flatMap((value, index) => /^(?:notes?|conditions?|settings?|harness|evaluation)$/i.test(value) ? [index] : []);
      continue;
    }
    if (!columns.length || !values[0] || values[0].length > 160 || !benchmarkInScope(values[0], question)
      || values.some(value => /not comparable|different harnesses/i.test(value))) continue;
    const scores = columns.flatMap(({ index, model }) => {
      const value = values[index];
      return value && value.length <= 60 && /^[-+]?\d[\d.,]*(?:\s*(?:%|Elo|ms|s|seconds?|points?|tokens\/s|tok\/s)|\s*\/\s*\d+(?:\.\d+)?)?(?:\s*\([^|]+\))?[*†‡]*$/i.test(value)
        ? [{ model, value }] : [];
    });
    const note = noteColumns.map(index => values[index]).filter(Boolean).join('; ').slice(0, 350);
    if (scores.length) rows.push({ name: values[0], scores, ...(note ? { note } : {}) });
    if (rows.length >= requestedBenchmarkCount(question)) break;
  }
  // Many publisher charts use divs rather than tables. Accept only labelled
  // metric blocks with adjacent model/value pairs, never unlabelled numbers.
  const blocks = lines.map(clean).filter(Boolean);
  for (let index = 0; index < blocks.length && rows.length < requestedBenchmarkCount(question); index++) {
    const heading = blocks[index].match(/^(.{2,140}?)\s+(?:Accuracy|Pass rate|Success rate|Elo rating|Score)\b(.+)$/i);
    if (!heading || !benchmarkInScope(heading[1], question)) continue;
    const unit = /%/.test(blocks[index]) ? '%' : /Elo rating/i.test(blocks[index]) ? ' Elo' : '';
    if (!unit) continue;
    const scores: BenchmarkRow['scores'] = [];
    let note: string | undefined;
    for (let cursor = index + 1; cursor < Math.min(index + 45, blocks.length); cursor++) {
      if (/^Source:/i.test(blocks[cursor])) {
        if (/effort|with(?:out)? tools|harness|evaluated|partial/i.test(blocks[cursor])) note = blocks[cursor].slice(0, 350);
        break;
      }
      if (/^#|(?:Accuracy|Elo rating|Pass rate)/i.test(blocks[cursor])) break;
      const model = blocks[cursor], value = blocks[cursor + 1] ?? '';
      if (model.length <= 100 && wanted(model) && /^\d[\d.,]*$/.test(value)) scores.push({ model, value: value + unit });
    }
    if (scores.length) rows.push({ name: heading[1].trim(), scores, ...(note ? { note } : {}) });
  }
  return rows;
}

export function benchmarkSentence(row: BenchmarkRow): string {
  return `Benchmark ${row.name}: ${row.scores.map(score => `${score.model} ${score.value}`).join('; ')}.${row.note ? ` ${row.note}` : ''}`;
}

/** Copy explicit score assertions only; a pronoun needs one named source subject. */
export function proseBenchmarkRow(text: string, title: string, question: string): BenchmarkRow | undefined {
  const subjects = comparisonSubjects(question);
  if (subjects.length !== 2) return;
  const metric = '(\\d+(?:[.,]\\d+)*(?:\\s*%|\\s+Elo|\\s*\\/\\s*\\d+(?:\\.\\d+)?))';
  const forward = text.match(new RegExp(`^(.{2,100}?)\\s+(?:scores?|scored|achieves?|achieved|reaches?|reached)\\s+${metric}\\s+on\\s+(.{2,100}?)(?=[,;]|\\.(?:\\s|$)|$)`, 'i'));
  const reverse = text.match(new RegExp(`^On\\s+(.{2,100}?),\\s+(.{2,100}?)\\s+(?:scores?|scored|achieves?|achieved|reaches?|reached)\\s+${metric}`, 'i'));
  if (!forward && !reverse) return;
  const actor = forward?.[1] ?? reverse![2];
  const name = (forward?.[3] ?? reverse![1]).trim();
  const value = forward?.[2] ?? reverse![3];
  const named = subjects.filter(subject => modelKey(actor) === modelKey(subject));
  const titleSubjects = subjects.filter(subject => mentionsName(title, subject));
  const model = named.length === 1 ? named[0] : /^(?:it|the model)$/i.test(actor) && titleSubjects.length === 1 ? titleSubjects[0] : undefined;
  if (!model || !benchmarkInScope(name, question)) return;
  const note = /(?:with(?:out)? tools|effort|harness|partial pass)/i.test(text) ? text : undefined;
  return { name, scores: [{ model, value }], ...(note ? { note } : {}) };
}

export function renderBenchmarkTable(rows: Array<BenchmarkRow & { sourceId: string }>, question: string): string {
  const grouped: Array<BenchmarkRow & { sourceIds: string[] }> = [];
  const unit = (value: string) => value.replace(/^[-+]?\d[\d.,]*/, '').trim().toLowerCase();
  for (const row of rows) {
    const compatible = grouped.find(other => normalizeLabel(other.name).toLowerCase() === normalizeLabel(row.name).toLowerCase()
      && other.note === row.note
      && other.scores.every(score => row.scores.every(next => modelKey(score.model) === modelKey(next.model)
        ? score.value === next.value : unit(score.value) === unit(next.value))));
    if (compatible) {
      for (const score of row.scores) if (!compatible.scores.some(existing => modelKey(existing.model) === modelKey(score.model))) compatible.scores.push(score);
      if (!compatible.sourceIds.includes(row.sourceId)) compatible.sourceIds.push(row.sourceId);
    } else grouped.push({ ...row, scores: [...row.scores], sourceIds: [row.sourceId] });
  }
  const names = [...new Set(grouped.map(row => normalizeLabel(row.name).toLowerCase()))].slice(0, requestedBenchmarkCount(question));
  const unique = grouped.filter(row => names.includes(normalizeLabel(row.name).toLowerCase()));
  const columns = unique.flatMap(row => row.scores.map(score => score.model))
    .filter((model, index, all) => all.findIndex(other => modelKey(other) === modelKey(model)) === index);
  for (const subject of comparisonSubjects(question)) {
    if (!columns.some(column => mentionsName(column, subject))) columns.push(subject);
  }
  const cell = (value: string) => value.replace(/\|/g, '\\|').replace(/[\r\n]/g, ' ');
  const hasNotes = unique.some(row => row.note);
  return [
    `| Benchmark | ${columns.map(cell).join(' | ')} | Source |${hasNotes ? ' Conditions |' : ''}`,
    `| --- | ${columns.map(() => '---').join(' | ')} | --- |${hasNotes ? ' --- |' : ''}`,
    ...unique.map(row => `| ${cell(row.name)} | ${columns.map(model => cell(row.scores.find(score => modelKey(score.model) === modelKey(model))?.value ?? 'Not found')).join(' | ')} | ${row.sourceIds.map(id => `[${id}]`).join(' ')} |${hasNotes ? ` ${cell(row.note ?? '')} |` : ''}`),
    '', 'Reported source values; benchmark versions and evaluation settings must match before drawing performance conclusions.',
    ...(names.length < requestedBenchmarkCount(question, 0)
      ? [`Found ${names.length} of ${requestedBenchmarkCount(question)} requested benchmark rows; the remaining scores were not verified.`] : []),
  ].join('\n');
}
