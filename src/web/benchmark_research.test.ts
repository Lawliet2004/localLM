import { expect, it } from 'vitest';
import { QueryPlanner } from './planning/query_planner';
import { runStagedResearch } from './staged_research';
import type { LLMProvider, SearchResult } from './types';

const currentDate = '2026-09-29';
const requestedComparison = 'Can you create a chart comparing benchmarks between GPT-6 Astra and sonnet 5.5? '
  + 'And that chart should be of the top 10 benchmarks related to coding and knowledge work. '
  + 'Here, comparing means mainly writing the score of these two models in a chart format. '
  + 'This is GPT-6 Astra, and this is Sonnet 5.5.';

function selectOfferedSentences(queries = 'GPT-6 Astra benchmarks\nSonnet 5.5 benchmarks\nGPT-6 Astra Sonnet 5.5 benchmarks'): LLMProvider {
  return { generate: async request => {
    if (request.systemPrompt.includes('search queries')) return { text: queries };
    if (request.systemPrompt.includes('fact extractor')) {
      return { text: [...request.userPrompt.matchAll(/^(S\d+\.\d+):/gm)].map(match => match[1]).join('\n') };
    }
    // Force the research pipeline to render the evidence it actually retained.
    return { text: '' };
  } };
}

function searchHit(snippet: string): SearchResult {
  return { id: 'benchmark-page', queryId: 'q', title: 'GPT-6 Astra and Sonnet 5.5 benchmark results',
    url: 'https://benchmarks.example/results', domain: 'benchmarks.example', rank: 1, snippet };
}

it('keeps the models explicitly named in the request when the same request also says these two models', async () => {
  const planner = new QueryPlanner(selectOfferedSentences());
  const queries = await planner.planQueries(requestedComparison, 'any', 4, currentDate);
  expect(queries.some(item => /GPT-6 Astra/i.test(item.query))).toBe(true);
  expect(queries.some(item => /Sonnet 5\.5/i.test(item.query))).toBe(true);

  const result = await runStagedResearch({
    question: requestedComparison, currentDate, model: selectOfferedSentences(),
    search: async () => ({ results: [], rawCount: 0, failureCount: 0, failures: [] }),
  });
  expect(result.clarificationQuestion).toBeUndefined();
  expect(result.queries.some(item => /GPT-6 Astra/i.test(item.query))).toBe(true);
  expect(result.queries.some(item => /Sonnet 5\.5/i.test(item.query))).toBe(true);
});

it('retains ten available benchmark rows spanning coding and knowledge work with their original score units', async () => {
  // Synthetic source data: these are retrieval fixtures, not real model scores.
  const rows = [
    ['SWE-bench Verified coding', '82.1%', '79.2%'],
    ['Terminal-Bench coding', '71.3%', '68.4%'],
    ['LiveCodeBench coding', '88.5%', '85.6%'],
    ['Aider programming', '76.7%', '74.8%'],
    ['Codeforces programming rating', '2450', '2310'],
    ['GPQA scientific knowledge', '90.1%', '89.2%'],
    ['MMLU knowledge', '91.3%', '90.4%'],
    ['GDPval business knowledge work', '68.5%', '64.6%'],
    ['Professional knowledge evaluation', '7.7 / 10', '7.2 / 10'],
    ['Office knowledge work', '81.8%', '78.9%'],
  ];
  const result = await runStagedResearch({
    question: 'Create a chart comparing the top 10 coding and knowledge work benchmark scores for GPT-6 Astra and Sonnet 5.5.',
    currentDate, model: selectOfferedSentences(),
    search: async () => ({ results: [searchHit('GPT-6 Astra and Sonnet 5.5 benchmark results ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'benchmarks.example', title: 'Benchmark results',
      text: '| Benchmark | GPT-6 Astra | Sonnet 5.5 |\n| --- | --- | --- |\n'
        + rows.map(row => `| ${row.join(' | ')} |`).join('\n'),
      retrievedAt: currentDate, searchResultIds: [] }),
  });
  for (const [name, astra, sonnet] of rows) {
    const fact = result.facts.find(item => item.text.includes(name));
    expect(fact, `Missing source row: ${name}`).toBeDefined();
    expect(fact?.text).toContain(astra);
    expect(fact?.text).toContain(sonnet);
  }
});

it('does not treat a generic model introduction as a retrieved benchmark score', async () => {
  const intro = 'GPT-6 Astra and Sonnet 5.5 are powerful models for many workflows.';
  const result = await runStagedResearch({
    question: 'GPT-6 Astra and Sonnet 5.5 benchmark scores', currentDate,
    model: selectOfferedSentences('GPT-6 Astra Sonnet 5.5 benchmarks'),
    search: async () => ({ results: [searchHit(intro)], rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.facts).toEqual([]);
  expect(result.missing.length).toBeGreaterThan(0);
  expect(result.queries.length).toBeGreaterThan(1);
});

it('does not mistake a cost percentage for a benchmark score', async () => {
  const result = await runStagedResearch({
    question: 'GPT-6 Astra benchmark scores', currentDate,
    model: selectOfferedSentences('GPT-6 Astra benchmarks'),
    search: async () => ({ results: [searchHit('GPT-6 Astra ties another model at 40% of the cost per task.')],
      rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.facts).toEqual([]);
});

it('does not select unrelated table columns when the second requested model is absent', async () => {
  const result = await runStagedResearch({
    question: 'GPT-6 Astra and Sonnet 5.5 coding benchmark scores', currentDate,
    model: selectOfferedSentences('GPT-6 Astra Sonnet 5.5 coding benchmarks'),
    search: async () => ({ results: [searchHit('GPT-6 Astra benchmark table ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'benchmarks.example', title: 'Model benchmarks',
      text: '| Benchmark | Older Alpha | Older Beta | GPT-6 Astra |\n| --- | --- | --- | --- |\n'
        + '| SWE-bench Verified coding | 41.2% | 45.3% | 82.1% |',
      retrievedAt: currentDate, searchResultIds: [] }),
  });
  expect(result.facts.some(fact => /^Benchmark.*Older Alpha 41\.2%; Older Beta 45\.3%/.test(fact.text))).toBe(false);
  expect(result.answer).not.toContain('Older Alpha 41.2%; Older Beta 45.3%');
});

it('preserves the original ten-row objective when the agent sends only one model keyword query', async () => {
  const result = await runStagedResearch({
    question: 'GPT-6 Astra benchmarks', objective: requestedComparison, currentDate,
    model: selectOfferedSentences('GPT-6 Astra benchmarks'),
    search: async () => ({ results: [searchHit('GPT-6 Astra benchmark results.')], rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'benchmarks.example', title: 'Model benchmarks',
      text: '| Benchmark | GPT-6 Astra | Sonnet 5.5 |\n| --- | --- | --- |\n'
        + Array.from({ length: 10 }, (_, i) => `| Coding evaluation ${i + 1} | ${80 + i}% | ${70 + i}% |`).join('\n'),
      retrievedAt: currentDate, searchResultIds: [] }),
  });
  expect(result.facts).toHaveLength(10);
  expect(result.answer).toContain('| Coding evaluation 10 | 89% | 79% |');
});

it('searches missing model scores using benchmark names discovered in an earlier source', async () => {
  const queries: string[] = [];
  const result = await runStagedResearch({
    question: 'Compare top 2 coding and knowledge work benchmarks between Alpha and Beta', currentDate,
    model: selectOfferedSentences('Alpha benchmarks\nBeta benchmarks\nAlpha vs Beta benchmarks'),
    search: async query => {
      queries.push(query);
      const targeted = /Alpha.*Terminal-Bench/.test(query);
      return { results: [{ ...searchHit('Alpha and Beta benchmark results'),
        url: targeted ? 'https://target.example/scores' : 'https://benchmarks.example/results' }],
      rawCount: 1, failureCount: 0, failures: [] };
    },
    fetchPage: async url => ({ id: 'page', url, domain: new URL(url).hostname, title: 'Scores',
      text: url.includes('target.example')
        ? '| Benchmark | Alpha | Beta |\n| Terminal-Bench | 80% | 70% |\n| GPQA | 90% | 85% |'
        : '| Benchmark | Beta |\n| Terminal-Bench | 70% |\n| GPQA | 85% |',
      retrievedAt: currentDate, searchResultIds: [] }),
  });
  expect(queries.some(query => /Alpha.*Terminal-Bench/.test(query))).toBe(true);
  expect(result.answer).toContain('| Terminal-Bench | 70% | 80% |');
  expect(result.answer).toContain('| GPQA | 85% | 90% |');
  expect(result.queries.length).toBeLessThanOrEqual(7);
});

it('keeps accepted prose scores alongside a partially populated comparison table', async () => {
  const result = await runStagedResearch({
    question: 'Compare Alpha and Beta benchmark scores', currentDate,
    model: selectOfferedSentences('Alpha benchmarks\nBeta benchmarks\nAlpha vs Beta benchmarks'),
    search: async query => ({ results: [{ ...searchHit(query === 'Alpha benchmarks'
      ? 'Alpha scored 88% on SWE-bench.' : 'Beta benchmark results'), url: `https://example.com/${query.startsWith('Alpha') ? 'alpha' : 'beta'}` }],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'example.com', title: 'Scores',
      text: url.endsWith('beta') ? '| Benchmark | Beta |\n| GPQA | 85% |' : 'Alpha scored 88% on SWE-bench.',
      retrievedAt: currentDate, searchResultIds: [] }),
  });
  expect(result.answer).toContain('Alpha scored 88% on SWE-bench.');
  expect(result.answer).toContain('| GPQA |');
});

it('reads another ranked source when the first two pages fail', async () => {
  const result = await runStagedResearch({
    question: 'Alpha benchmark scores', currentDate, model: selectOfferedSentences('Alpha benchmarks'),
    search: async () => ({ results: ['first', 'second', 'third'].map((name, index) => ({
      ...searchHit('Alpha benchmark results'), url: `https://${name}.example/scores`, rank: index + 1,
    })), rawCount: 3, failureCount: 0, failures: [] }),
    fetchPage: async url => {
      if (!url.includes('third')) throw new Error('HTTP 403');
      return { id: 'page', url, domain: 'third.example', title: 'Scores',
        text: '| Benchmark | Alpha |\n| SWE-bench | 81% |', retrievedAt: currentDate, searchResultIds: [] };
    },
  });
  expect(result.answer).toContain('81%');
  expect(result.fetchFailures).toBe(2);
});
