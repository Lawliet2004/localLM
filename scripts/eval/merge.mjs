// Combine eval reports (e.g. one per loaded model) into one comparison table.
//   node scripts/eval/merge.mjs test-results/eval/eval-A.json eval-B.json [--out merged.md]
import { readFileSync, writeFileSync } from 'node:fs';
import { renderMarkdown } from './report.mjs';

const args = process.argv.slice(2);
const outIndex = args.indexOf('--out');
const out = outIndex >= 0 ? args.splice(outIndex, 2)[1] : null;
if (!args.length) throw new Error('Pass one or more eval report JSON files.');
const reports = args.map(file => JSON.parse(readFileSync(file, 'utf8')));

const seeds = new Set(reports.map(report => report.seed));
const repeats = new Set(reports.map(report => report.repeats));
const merged = {
  startedAt: reports.map(report => report.startedAt).sort()[0],
  seed: seeds.size === 1 ? reports[0].seed : `mixed (${[...seeds].join(', ')})`,
  repeats: repeats.size === 1 ? reports[0].repeats : `mixed (${[...repeats].join(', ')})`,
  runs: reports.flatMap(report => report.runs),
  skipped: reports.flatMap(report => report.skipped ?? []),
};
const markdown = renderMarkdown(merged);
if (out) writeFileSync(out, markdown);
process.stdout.write(markdown);
