import { expect, it } from 'vitest';
import { extractBenchmarkRows, proseBenchmarkRow, renderBenchmarkTable } from './benchmark_table';

const comparison = 'Compare GPT-6 Astra and Sonnet 5.5 benchmark scores';

it('copies named prose benchmark scores and resolves pronouns only with an unambiguous source title', () => {
  expect(proseBenchmarkRow('GPT-6 Astra scored 81% on SWE-bench Verified.', 'Model scores', comparison))
    .toEqual({ name: 'SWE-bench Verified', scores: [{ model: 'GPT-6 Astra', value: '81%' }] });
  expect(proseBenchmarkRow('It scores 70.6% on Terminal-Bench 4.0, versus 10.3% previously.', 'Sonnet 5.5 released', comparison))
    .toEqual({ name: 'Terminal-Bench 4.0', scores: [{ model: 'Sonnet 5.5', value: '70.6%' }] });
  expect(proseBenchmarkRow('On CursorBench 4.0, it reaches 55.5% against another model.', 'Sonnet 5.5 released', comparison))
    .toEqual({ name: 'CursorBench 4.0', scores: [{ model: 'Sonnet 5.5', value: '55.5%' }] });
  expect(proseBenchmarkRow('It scores 70.6% on Terminal-Bench 4.0.', 'Sonnet 5.5 versus GPT-6 Astra', comparison)).toBeUndefined();
});

it('preserves table evaluation conditions and keeps different conditions separate', () => {
  const rows = extractBenchmarkRows('| Benchmark | GPT-6 Astra | Sonnet 5.5 | Conditions |\n'
    + '| SWE-bench | 81% | 79% | With tools, high effort |', comparison);
  expect(rows[0].note).toBe('With tools, high effort');
  const table = renderBenchmarkTable([
    { ...rows[0], sourceId: 'S1' },
    { name: 'SWE-bench', scores: [{ model: 'GPT-6 Astra', value: '81%' }], note: 'No tools', sourceId: 'S2' },
  ], comparison);
  expect(table).toContain('With tools, high effort');
  expect(table).toContain('No tools');
});

it('reads labelled chart blocks without assigning scores to the adjacent model', () => {
  const rows = extractBenchmarkRows('Terminal-Bench 4.0    Accuracy, % (higher is better)\n\n'
    + 'Claude Sonnet 5.5\n\n</div>\n\n70.6\n\n</div>\n\n'
    + 'Other model\n\n91.2\n\nGPT‑6 Astra\n\n57.9\n\nSource: Published benchmark report.', comparison);
  expect(rows).toEqual([{ name: 'Terminal-Bench 4.0', scores: [
    { model: 'Claude Sonnet 5.5', value: '70.6%' }, { model: 'GPT‑6 Astra', value: '57.9%' },
  ] }]);
});

it('normalizes presentation dashes without changing model versions', () => {
  expect(extractBenchmarkRows('| Benchmark | GPT‑6 Astra | Sonnet 5.5 |\n| SWE-bench | 81% | 79% |', comparison))
    .toHaveLength(1);
});

it('combines separately sourced model cells without counting one benchmark twice', () => {
  const answer = renderBenchmarkTable([
    { name: 'SWE-bench Verified', scores: [{ model: 'GPT-6 Astra', value: '81%' }], sourceId: 'S1' },
    { name: 'SWE-bench Verified', scores: [{ model: 'Sonnet 5.5', value: '79%' }], sourceId: 'S2' },
  ], 'Compare top 2 benchmarks between GPT-6 Astra and Sonnet 5.5');
  expect(answer).toContain('| SWE-bench Verified | 81% | 79% | [S1] [S2] |');
  expect(answer).toContain('1 of 2');
});

it('matches exact requested versions without taking similarly named model variants', () => {
  const rows = extractBenchmarkRows(
    '| Benchmark | GPT-6 Astra Mini | Sonnet 5.5 Preview | Sonnet 5.5.1 | GPT-6 Astra | Sonnet 5.5 |\n'
      + '| --- | --- | --- | --- | --- | --- |\n'
      + '| SWE-bench Verified | 41% | 42% | 43% | 81% | 79% |', comparison);
  expect(rows).toEqual([{ name: 'SWE-bench Verified', scores: [
    { model: 'GPT-6 Astra', value: '81%' },
    { model: 'Sonnet 5.5', value: '79%' },
  ] }]);
});

it('does not collapse a parenthesized model variant into the requested release', () => {
  expect(extractBenchmarkRows('| Benchmark | GPT-6 Astra (Mini) | Sonnet 5.5 (Preview) |\n'
    + '| SWE-bench | 41% | 42% |', comparison)).toEqual([]);
});

it('preserves explicit benchmark units and formatting instead of discarding measured results', () => {
  const rows = extractBenchmarkRows(
    '| Benchmark | GPT-6 Astra | Sonnet 5.5 |\n| --- | --- | --- |\n'
      + '| Codeforces rating | 2,450 Elo | 2,310 Elo |\n'
      + '| Task completion latency | 1,240 ms | 1,580 ms |\n'
      + '| Professional knowledge | 7.7 / 10 | 7.2 / 10 |', comparison);
  expect(rows).toEqual([
    { name: 'Codeforces rating', scores: [{ model: 'GPT-6 Astra', value: '2,450 Elo' }, { model: 'Sonnet 5.5', value: '2,310 Elo' }] },
    { name: 'Task completion latency', scores: [{ model: 'GPT-6 Astra', value: '1,240 ms' }, { model: 'Sonnet 5.5', value: '1,580 ms' }] },
    { name: 'Professional knowledge', scores: [{ model: 'GPT-6 Astra', value: '7.7 / 10' }, { model: 'Sonnet 5.5', value: '7.2 / 10' }] },
  ]);
});

it('reads a single-model table with Markdown bold headers and shows the missing counterpart', () => {
  const rows = extractBenchmarkRows(
    '| **Benchmark** | **GPT-6 Astra** |\n| --- | --- |\n'
      + '| **SWE-bench Verified** | **81.2%** |', comparison);
  expect(rows).toEqual([{ name: 'SWE-bench Verified', scores: [{ model: 'GPT-6 Astra', value: '81.2%' }] }]);
  const table = renderBenchmarkTable(rows.map(row => ({ ...row, sourceId: 'S1' })), comparison);
  expect(table).toContain('| Benchmark | GPT-6 Astra | Sonnet 5.5 | Source |');
  expect(table).toContain('| SWE-bench Verified | 81.2% | Not found | [S1] |');
});

it('extracts a single-model request without needing a comparison subject pair', () => {
  const rows = extractBenchmarkRows(
    '| Benchmark | **GPT-6 Astra** | Other model |\n| --- | --- | --- |\n'
      + '| GPQA | 88.2% | 75% |', 'Show GPT-6 Astra benchmark scores');
  expect(rows).toEqual([{ name: 'GPQA', scores: [{ model: 'GPT-6 Astra', value: '88.2%' }] }]);
});

it('keeps ten requested source rows across coding and knowledge work instead of the default three', () => {
  const names = ['SWE-bench', 'Terminal-Bench', 'LiveCodeBench', 'Aider', 'Codeforces',
    'GPQA', 'MMLU', 'GDPval', 'Professional knowledge', 'Office knowledge work'];
  const question = 'Compare GPT-6 Astra and Sonnet 5.5 on the top 10 coding and knowledge work benchmarks';
  const rows = extractBenchmarkRows('| Benchmark | GPT-6 Astra | Sonnet 5.5 |\n| --- | --- | --- |\n'
    + names.map((name, index) => `| ${name} | ${80 + index}% | ${70 + index}% |`).join('\n'), question);
  expect(rows.map(row => row.name)).toEqual(names);
  const table = renderBenchmarkTable(rows.map(row => ({ ...row, sourceId: 'S1' })), question);
  expect(table).toContain('| Office knowledge work | 89% | 79% | [S1] |');
  expect(table).not.toContain('remaining scores were not verified');
});

it('does not turn rows explicitly marked as different evaluation harnesses into comparisons', () => {
  const rows = extractBenchmarkRows('| Benchmark | GPT-6 Astra | Sonnet 5.5 | Notes |\n| --- | --- | --- | --- |\n'
    + '| SWE-bench | 81% | 79% | Different harnesses |\n'
    + '| GPQA | 88% | 86% | Not comparable |', comparison);
  expect(rows).toEqual([]);
});
