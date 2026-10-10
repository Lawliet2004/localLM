import { expect, it, vi } from 'vitest';
import { filterSnippetResults, runStagedResearch } from './staged_research';
import type { GenerationRequest, SearchResult } from './types';
import { WebSearchEngine } from './index';
import { RESEARCH_BUDGETS } from './config/defaults';
import { QueryPlanner } from './planning/query_planner';

const hit = (url: string, snippet: string, rank = 1): SearchResult => ({
  id: url, queryId: 'q', title: 'Result title', url, domain: new URL(url).hostname, rank, snippet,
});

it('returns collected evidence when a later query exceeds the total deadline', async () => {
  vi.useFakeTimers();
  let count = 0;
  let secondQuery!: () => void;
  const reachedSecondQuery = new Promise<void>(resolve => { secondQuery = resolve; });
  const options = {
    question: 'adapter charging and warranty', currentDate: '2026-09-30', deadlineMs: Date.now() + 1000,
    model: { generate: async (request: GenerationRequest) => ({ text: request.systemPrompt.includes('search queries') ? 'adapter charging\nadapter warranty' : 'S1.1' }) },
    search: async () => {
      if (++count > 1) { secondQuery(); return new Promise<never>(() => {}); }
      return { results: [hit('https://spec.example/adapter', 'Adapter charging supplies 60 watts.')], rawCount: 1, failureCount: 0, failures: [] };
    },
  };
  try {
    const pending = runStagedResearch(options);
    await reachedSecondQuery;
    await vi.advanceTimersByTimeAsync(1000);
    const result = await pending;
    expect(result.answer).toContain('60 watts'); expect(result.facts).toHaveLength(1);
    expect(result.failures.some(failure => /deadline/i.test(failure.error))).toBe(true);
  } finally { vi.useRealTimers(); }
}, 1000);

it('rejects synthesis with reversed polarity and accepts faithful text', async () => {
  for (const synthesis of ['The adapter does not support USB-C charging at up to 60 watts. [S1]', 'The adapter supports USB-C charging at up to 60 watts. [S1]']) {
    const result = await runStagedResearch({
      question: 'adapter USB-C charging', currentDate: '2026-09-30',
      model: { generate: async request => ({ text: request.systemPrompt.includes('search queries') ? 'adapter USB-C charging'
        : request.systemPrompt.includes('fact extractor') ? 'S1.1' : synthesis }) },
      search: async () => ({ results: [hit('https://spec.example/adapter', 'The adapter supports USB-C charging at up to 60 watts.')], rawCount: 1, failureCount: 0, failures: [] }),
    });
    expect(result.answer).not.toContain('does not support');
    expect(result.answer).toContain('supports USB-C charging');
  }
});

it('retains table evidence when the prose model throws', async () => {
  const result = await runStagedResearch({
    question: 'Alpha vs Beta coding benchmarks', currentDate: '2026-09-30',
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'Alpha vs Beta coding benchmarks' };
      throw new Error('model unavailable');
    } },
    search: async () => ({ results: [hit('https://table.example/results', 'Alpha vs Beta coding benchmarks.'),
      hit('https://prose.example/results', 'Alpha scored 70% on coding benchmark.')], rawCount: 2, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: new URL(url).hostname, title: 'Coding benchmarks',
      text: url.includes('table.example') ? '| Benchmark | Alpha | Beta |\n| --- | --- | --- |\n| SWE-bench Verified | 70% | 60% |' : 'Alpha scored 70% on coding benchmark.',
      retrievedAt: '2026-09-30', searchResultIds: [] }),
  });
  expect(result.facts.some(fact => fact.benchmarkRow)).toBe(true);
  expect(result.answer).toContain('70%');
  expect(result.extractionDiagnostics.some(item => item.outcome === 'model_error')).toBe(true);
});

it('keeps two distinct relevant URLs per domain and rejects a third', () => {
  const results = [1, 2, 3].map(i => hit(`https://spec.example/page${i}`, 'Adapter charging specifications.', i));
  expect(filterSnippetResults([...results, results[0]], 'adapter charging').map(item => item.url))
    .toEqual(['https://spec.example/page1', 'https://spec.example/page2']);
});

it('continues an independent query after a provider failure and retains partial evidence', async () => {
  const result = await runStagedResearch({
    question: 'Node.js release and SQLite release', currentDate: '2026-09-29',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js release\nSQLite release' : request.systemPrompt.includes('fact extractor')
        ? 'S1.1' : 'unsupported answer' }) },
    search: async query => query.includes('Node.js')
      ? { results: [], rawCount: 0, failureCount: 1, failures: [{ query, error: 'fetch failed (ECONNRESET)' }] }
      : { results: [hit('https://sqlite.org/release', 'SQLite release is 3.50.0.')], rawCount: 1, failureCount: 0, failures: [] },
  });
  expect(result.queries.map(q => q.query)).toContain('SQLite release');
  expect(result.facts[0]?.text).toBe('SQLite release is 3.50.0.');
  expect(result.providerUnreachable).toBe(false);
  expect(result.missing).toContain('Node.js release');
});

it('selects a late page passage for the current keyword query', async () => {
  let extraction = '';
  const result = await runStagedResearch({
    question: 'Node.js release downloads installation documentation support and license', currentDate: '2026-09-28',
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'Node.js license details' };
      if (request.systemPrompt.includes('fact extractor')) {
        extraction = request.userPrompt;
        return { text: 'S1: Node.js license is MIT.' };
      }
      return { text: 'Node.js license is MIT. [S1]' };
    } },
    search: async () => ({ results: [hit('https://nodejs.org/about', 'Node.js documentation and downloads.')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'nodejs.org', title: 'Node.js',
      text: 'Node.js release downloads installation documentation support. ' + 'General introduction. '.repeat(1100) + 'Node.js license is MIT.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  expect(extraction).toContain('Node.js license is MIT.');
  expect(extraction.length).toBeLessThan(2000);
  expect(result.facts[0]?.text).toBe('Node.js license is MIT.');
});

it('passes at most five short snippets with at most two per domain to extraction', () => {
  const results = [hit('https://one.example/a', 'First fact. Second fact. Third fact.'),
    hit('https://one.example/b', 'Duplicate domain.'),
    ...Array.from({ length: 8 }, (_, i) => hit(`https://site${i}.example/x`, 'Useful fact.'))];
  const filtered = filterSnippetResults(results);
  expect(filtered).toHaveLength(5);
  expect(new Set(filtered.map(item => item.domain)).size).toBe(4);
  expect(filtered[0].snippet).toBe('First fact. Second fact.');
  expect(Object.keys(filtered[0]).sort()).toEqual(['domain', 'snippet', 'title', 'url']);
});

it('ranks relevant hits before enforcing the one-domain limit', () => {
  const results = [
    hit('https://nodejs.org/unrelated', 'General developer resources.', 1),
    hit('https://nodejs.org/releases', 'Node.js version 24.1.0 release notes.', 3),
    hit('https://other.example/story', 'Football scores.', 2),
  ];
  const filtered = filterSnippetResults(results, 'Node.js latest version 2026');
  expect(filtered[0].url).toBe('https://nodejs.org/releases');
  expect(filtered.every(item => item.url !== 'https://other.example/story')).toBe(true);
});

it('prefers a matching official domain over an equally relevant repost', () => {
  const results = [
    hit('https://blog.example/node-release', 'Node.js version 24 release notes.', 1),
    hit('https://nodejs.org/releases', 'Node.js version 24 release notes.', 3),
  ];
  const filtered = filterSnippetResults(results, 'Node.js latest version 2026');
  expect(filtered[0].domain).toBe('nodejs.org');
});

it('prioritizes the model publisher for a benchmark search', () => {
  const filtered = filterSnippetResults([
    hit('https://blog.example/opus', 'Opus 5.5 coding benchmark score 66.4%.', 1),
    hit('https://www.anthropic.com/claude-opus-5-5', 'Opus 5.5 agentic coding benchmark results.', 8),
  ], 'Opus 5.5 coding benchmark 2026');
  expect(filtered[0].domain).toBe('anthropic.com');
});

it('extracts from a bounded page passage when a search snippet lacks the answer', async () => {
  const prompts: GenerationRequest[] = [];
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-28',
    model: { generate: async request => {
      prompts.push(request);
      return { text: request.systemPrompt.includes('search queries') ? 'Node.js latest version 2026'
        : request.systemPrompt.includes('fact extractor') ? '- [S1] Node.js version 24.1.0 is current.'
          : 'Node.js version 24.1.0 is current. [S1]' };
    } },
    search: async () => ({ results: [hit('https://nodejs.org/releases', 'Node.js release notes and downloads.')], rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'nodejs.org', title: 'Node.js releases',
      text: 'About releases. Node.js version 24.1.0 is current. Older releases are listed below.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  expect(prompts.find(request => request.systemPrompt.includes('fact extractor'))?.userPrompt).toContain('Node.js version 24.1.0 is current.');
  expect(result.sources.S1.snippet).toContain('24.1.0');
  expect(result.sources.S1.evidenceKind).toBe('page_excerpt');
  expect(result.pagesFetched).toBe(1);
});

it('keeps a relevant search snippet when the fetched page excerpt is generic', async () => {
  const prompts: GenerationRequest[] = [];
  const result = await runStagedResearch({
    question: 'GPT-6 Astra benchmark score?', currentDate: '2026-09-28',
    model: { generate: async request => {
      prompts.push(request);
      return { text: request.systemPrompt.includes('search queries') ? 'GPT-6 Astra benchmark score'
        : request.systemPrompt.includes('fact extractor') ? '- [S1] GPT-6 Astra scored 64.6% on Terminal-Bench.'
          : 'GPT-6 Astra scored 64.6% on Terminal-Bench. [S1]' };
    } },
    search: async () => ({ results: [hit('https://official.example/benchmarks',
      'GPT-6 Astra scored 64.6% on Terminal-Bench.')], rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'official.example', title: 'GPT-6 Astra benchmarks',
      text: 'GPT-6 Astra benchmark overview. Read the model introduction and deployment notes.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  const extraction = prompts.find(request => request.systemPrompt.includes('fact extractor'))?.userPrompt;
  expect(extraction).toContain('GPT-6 Astra scored 64.6% on Terminal-Bench.');
  expect(extraction).toContain('GPT-6 Astra benchmark overview.');
  expect(result.facts[0]?.text).toBe('GPT-6 Astra scored 64.6% on Terminal-Bench.');
});

it('reuses fetched page evidence when the same URL appears on retry', async () => {
  const extractionPrompts: string[] = [];
  const fetchPage = vi.fn(async (url: string) => ({ id: 'page', url, domain: 'bench.example', title: 'Comparison',
    text: 'Alpha scored 66.4% on Terminal-Bench coding.',
    retrievedAt: '2026-09-28', searchResultIds: [] }));
  const result = await runStagedResearch({
    question: 'Alpha coding benchmark score?', currentDate: '2026-09-28',
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'Alpha coding benchmark 2026' };
      if (request.systemPrompt.includes('fact extractor')) {
        extractionPrompts.push(request.userPrompt);
        return { text: extractionPrompts.length === 1 ? 'NO_ANSWER_FOUND' : 'S2.2' };
      }
      return { text: 'unsupported prose' };
    } },
    search: async () => ({ results: [hit('https://bench.example/compare', 'Alpha coding benchmarks ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage,
  });
  expect(fetchPage).toHaveBeenCalledTimes(1);
  expect(extractionPrompts[1]).toContain('Alpha scored 66.4% on Terminal-Bench coding.');
  expect(result.pagesFetched).toBe(1);
  expect(result.facts).toHaveLength(1);
});

it('retries an unanswered query and synthesizes only extracted, cited facts', async () => {
  const calls: GenerationRequest[] = [];
  const searches: string[] = [];
  const model = { generate: vi.fn(async (request: GenerationRequest) => {
    calls.push(request);
    if (request.systemPrompt.includes('search queries')) return { text: 'Node.js latest version 2026' };
    if (request.systemPrompt.includes('fact extractor')) return { text: searches.length === 1 ? 'NO_ANSWER_FOUND' : '- [S1] Node.js 24.1.0 is the current release.' };
    return { text: 'Node.js 24.1.0 is the current release. [S1]' };
  }) };
  const result = await runStagedResearch({
    question: 'What is the latest version of Node.js?', currentDate: '2026-09-27', model,
    search: async query => {
      searches.push(query);
      return { results: [hit(`https://release${searches.length}.example/`, searches.length === 1
        ? 'Older release notes only.' : 'Node.js 24.1.0 is the current release.')], rawCount: 1, failureCount: 0, failures: [] };
    },
  });
  expect(searches).toHaveLength(3); // one empty attempt, one answer, one bounded corroboration search
  expect(searches[0]).toBe('Node.js latest version 2026');
  expect(searches[1]).not.toBe(searches[0]);
  expect(calls).toHaveLength(3); // plan and two extractions; the exact fact needs no synthesis
  expect(result.answer).toContain('Node.js 24.1.0');
  expect(result.answer).toContain('https://release2.example/');
  expect(result.facts).toHaveLength(1);
});

it('broadens a failed current query and removes the time filter on retry', async () => {
  const requests: Array<{ query: string; freshness: string | undefined }> = [];
  await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-28', freshness: 'month',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : 'NO_ANSWER_FOUND' }) },
    search: async (query, freshness) => {
      requests.push({ query, freshness });
      return { results: [], rawCount: 0, failureCount: 0, failures: [] };
    },
  });
  expect(requests).toHaveLength(3);
  expect(requests.map(request => request.freshness)).toEqual(['month', 'any', 'any']);
  expect(requests[2].query).not.toContain('2026');
});

it('keeps the coding benchmark constraint through retries', async () => {
  const searched: string[] = [];
  const result = await runStagedResearch({
    question: 'Alpha coding benchmark score?', currentDate: '2026-09-28',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Alpha coding benchmark 2026' : 'S1.1' }) },
    search: async query => {
      searched.push(query);
      return { results: [hit(`https://site${searched.length}.example/`,
        'Alpha released on September 22, 2026.')], rawCount: 1, failureCount: 0, failures: [] };
    },
  });
  expect(searched).toHaveLength(3);
  expect(searched.every(query => query.includes('coding'))).toBe(true);
  expect(result.facts).toHaveLength(0);
});

it('stops semantic retries when the search provider is unreachable', async () => {
  const search = vi.fn(async () => ({ results: [], rawCount: 0, failureCount: 1, failures: [{ query: 'q', error: 'fetch failed' }] }));
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-27',
    model: { generate: async () => ({ text: 'Node.js latest version 2026' }) }, search,
  });
  expect(search).toHaveBeenCalledTimes(1);
  expect(result.providerUnreachable).toBe(true);
  expect(result.facts).toHaveLength(0);
});

it('rejects invented facts even when the model attaches a real source ID', async () => {
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-27',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : '- [S1] Node.js version 99.0.0 is current.' }) },
    search: async () => ({ results: [hit('https://node.example/release', 'Node.js version 24.1.0 is current.')], rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.facts).toHaveLength(0);
  expect(result.sources).toEqual({});
  expect(result.queries).toHaveLength(3);
  expect(result.extractionDiagnostics.map(item => item.outcome)).toEqual([
    'unverified_output', 'unverified_output', 'unverified_output',
  ]);
});

it('records a truncated empty extractor response for the next failure diagnosis', async () => {
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-28',
    model: { generate: async request => request.systemPrompt.includes('search queries')
      ? { text: 'Node.js latest version 2026' } : { text: '', finishReason: 'length' } },
    search: async () => ({ results: [hit('https://node.example/release', 'Node.js version 24.1.0 is current.')],
      rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.extractionDiagnostics[0]).toMatchObject({
    outcome: 'unverified_output', finishReason: 'length', outputChars: 0,
  });
});

it('accepts an exact source phrase with a simple source prefix', async () => {
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-27',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : request.systemPrompt.includes('fact extractor')
        ? 'S1: Node.js version 24.1.0 is current.' : 'Node.js version 24.1.0 is current. [S1]' }) },
    search: async () => ({ results: [hit('https://node.example/release', 'Node.js version 24.1.0 is current.')],
      rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.facts).toHaveLength(1);
  expect(result.extractionDiagnostics[0].outcome).toBe('accepted');
});

it('copies selected evidence sentences instead of trusting model-written facts', async () => {
  let extractionPrompt = '';
  const result = await runStagedResearch({
    question: 'GPT-6 Astra benchmark score?', currentDate: '2026-09-28', maxQueries: 1,
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'GPT-6 Astra benchmark score' };
      if (request.systemPrompt.includes('fact extractor')) {
        extractionPrompt = request.userPrompt;
        return { text: '<think>S1.1 looks generic.</think>\nS1.2' };
      }
      return { text: 'GPT-6 Astra scored 64.6% on Terminal-Bench. [S1]' };
    } },
    search: async () => ({ results: [hit('https://official.example/benchmarks',
      'General model introduction. GPT-6 Astra scored 64.6% on Terminal-Bench.')],
      rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(extractionPrompt).toContain('S1.2: GPT-6 Astra scored 64.6% on Terminal-Bench.');
  expect(result.facts).toEqual([{ text: 'GPT-6 Astra scored 64.6% on Terminal-Bench.',
    sourceId: 'S1', query: 'GPT-6 Astra benchmark score' }]);
  expect(result.answer).toContain('64.6%');
});

it('keeps comparable coding benchmark table rows intact', async () => {
  let extractionPrompt = '';
  const result = await runStagedResearch({
    question: 'Alpha and Beta coding benchmark scores', currentDate: '2026-09-28',
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'Alpha Beta coding benchmarks' };
      if (request.systemPrompt.includes('fact extractor')) {
        extractionPrompt = request.userPrompt;
        return { text: 'S1.2\nS1.3' };
      }
      return { text: 'unsupported prose' };
    } },
    search: async () => ({ results: [hit('https://bench.example/compare', 'Alpha and Beta coding benchmarks ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'bench.example', title: 'Comparison',
      text: 'Intro.\n\n| Benchmark | Alpha | Beta | Winner |\n\n| Terminal-Bench coding | 66.4% | 57.9% | Alpha |\n\n| FrontierCode agentic coding | 54.4% | 53.3% | Alpha |\n\n| Terminal-Bench-Science scientific research | 58.7% | 64.6% | Beta |\n\nEnd.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  expect(extractionPrompt).toBe(''); // Named table cells are copied without model selection.
  expect(result.answer).not.toContain('Terminal-Bench-Science');
  expect(result.facts.map(fact => fact.text)).toEqual([
    'Benchmark Terminal-Bench coding: Alpha 66.4%; Beta 57.9%.',
    'Benchmark FrontierCode agentic coding: Alpha 54.4%; Beta 53.3%.',
  ]);
  expect(result.answer).toContain('| Terminal-Bench coding | 66.4% | 57.9% |');
});

it('reads benchmark rows from a publisher table with extra model columns', async () => {
  const result = await runStagedResearch({
    question: 'Opus 5.5 GPT-6 Astra coding benchmark scores', currentDate: '2026-09-28',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Opus 5.5 GPT-6 Astra coding benchmark' : request.systemPrompt.includes('fact extractor')
        ? 'S1.2' : 'unsupported prose' }) },
    search: async () => ({ results: [hit('https://publisher.example/benchmarks', 'Opus 5.5 GPT-6 Astra coding benchmarks ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'publisher.example', title: 'Benchmarks',
      text: 'Intro.\n\n|  | Opus 5.5 | Opus 5 | GPT-6 Astra |\n\n| Agentic coding Terminal-Bench 4.0 | 66.4% | 44.0% | 57.9% |\n\nEnd.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  expect(result.facts[0]?.text).toBe('Benchmark Agentic coding Terminal-Bench 4.0: Opus 5.5 66.4%; GPT-6 Astra 57.9%.');
});

it('uses a shared benchmark row to answer a comparison despite empty side searches', async () => {
  const result = await runStagedResearch({
    question: 'Compare Opus 5.5 and GPT-6 Astra coding benchmark scores.', currentDate: '2026-09-28',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Opus 5.5 coding benchmark\nGPT-6 Astra coding benchmark\nOpus 5.5 vs GPT-6 Astra coding benchmark'
      : request.systemPrompt.includes('fact extractor')
        ? request.userPrompt.includes('Search query: GPT-6 Astra coding benchmark') ? 'S4.2' : 'NO_ANSWER_FOUND'
        : 'unsupported prose' }) },
    search: async () => ({ results: [hit('https://publisher.example/benchmarks', 'Coding benchmark table ...')],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: 'page', url, domain: 'publisher.example', title: 'Benchmarks',
      text: '|  | Opus 5.5 | Opus 5 | GPT-6 Astra |\n| Agentic coding Terminal-Bench 4.0 | 66.4% | 44.0% | 57.9% |\nEnd.',
      retrievedAt: '2026-09-28', searchResultIds: [] }),
  });
  expect(result.missing).toEqual([]);
  expect(result.answer).toContain('| Agentic coding Terminal-Bench 4.0 | 66.4% | 57.9% |');
  expect(result.answer).not.toContain('Could not find:');
});

it('falls back to extracted facts when synthesis changes a version number', async () => {
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-09-27',
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : request.systemPrompt.includes('fact extractor')
        ? '- [S1] Node.js version 24.1.0 is current.' : 'Node.js version 24.2.0 is current. [S1]' }) },
    search: async () => ({ results: [hit('https://node.example/release', 'Node.js version 24.1.0 is current.')], rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.answer).toContain('24.1.0');
  expect(result.answer).not.toContain('24.2.0');
});

it('does not invent a benchmark winner from separate cited scores', async () => {
  const model = { generate: async (request: GenerationRequest) => {
    if (request.systemPrompt.includes('search queries')) return { text: 'Llama 3 benchmark scores\nMistral 7B benchmark scores\nLlama 3 vs Mistral 7B benchmark scores' };
    if (request.systemPrompt.includes('fact extractor')) {
      const id = request.userPrompt.match(/\[(S\d+)\]/)?.[1];
      return { text: `- [${id}] ${request.userPrompt.includes('Llama 3 score is 70') ? 'Llama 3 score is 70.' : 'Mistral 7B score is 60.'}` };
    }
    return { text: 'Llama 3 scores higher than Mistral 7B. [S1][S2]' };
  } };
  const result = await runStagedResearch({
    question: 'Compare benchmarks between Llama 3 and Mistral 7B.', currentDate: '2026-09-28', model,
    search: async query => ({ results: [hit(`https://${query.startsWith('Llama') ? 'llama' : 'mistral'}.example/scores`,
      query.startsWith('Llama') ? 'Llama 3 score is 70.' : 'Mistral 7B score is 60.')], rawCount: 1, failureCount: 0, failures: [] }),
  });
  expect(result.answer).not.toMatch(/scores higher/i);
  expect(result.answer).toContain('Llama 3 score is 70');
});

it('resolves two named models from recent chat context before planning queries', async () => {
  const searches: string[] = [];
  const model = { generate: async (request: GenerationRequest) => ({ text: request.systemPrompt.includes('reference resolver')
    ? 'Llama 3\nMistral 7B' : request.systemPrompt.includes('search queries')
      ? 'Llama 3 benchmark scores\nMistral 7B benchmark scores\nLlama 3 vs Mistral 7B scores'
      : request.systemPrompt.includes('fact extractor') ? 'NO_ANSWER_FOUND' : '' }) };
  await runStagedResearch({
    question: 'Compare benchmarks between these two models.',
    context: 'Earlier we discussed Llama 3 and Mistral 7B.',
    currentDate: '2026-09-28', model,
    search: async query => { searches.push(query); return { results: [], rawCount: 0, failureCount: 0, failures: [] }; },
  });
  expect(searches).toContain('Llama 3 benchmark scores');
  expect(searches).toContain('Mistral 7B benchmark scores');
  expect(searches.some(query => query.includes('these two models'))).toBe(false);
});

it('asks for the missing model names instead of searching unresolved pronouns', async () => {
  const search = vi.fn();
  const result = await runStagedResearch({
    question: 'Compare benchmarks between these two models.', currentDate: '2026-09-28',
    model: { generate: async () => ({ text: 'irrelevant' }) }, search,
  });
  expect(result.answer).toMatch(/which two models/i);
  expect(search).not.toHaveBeenCalled();
});

it('exposes staged evidence in the worker-compatible research session shape', async () => {
  const engine = new WebSearchEngine({
    llmProvider: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : request.systemPrompt.includes('fact extractor')
        ? '- [S1] Node.js 24.1.0 is current.' : 'Node.js 24.1.0 is current. [S1]' }) },
    searchProvider: { search: async () => [hit('https://release.example/version', 'Node.js 24.1.0 is current.')] },
  });
  const session = await engine.researchStaged('What is the latest Node.js version?', { currentDate: '2026-09-27' });
  expect(session.sources.S1.url).toBe('https://release.example/version');
  expect(session.documents[0].metadata?.extractionMethod).toBe('search_snippet');
  expect(session.answer).toContain('https://release.example/version');
  expect(session.trace.queriesGenerated).toBe(2);
  expect(session.evidence[0].assessment).toBe('snippet_only');
  expect(session.verification?.supportedCount).toBe(1);
});

it('retains corroborating excerpts from separate source domains', async () => {
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license' : request.systemPrompt.includes('fact extractor') ? 'S1.1\nS2.1' : 'Node.js license is MIT. [S1][S2]' }) },
    search: async () => ({ results: [hit('https://nodejs.org/about', 'Node.js license is MIT.'),
      hit('https://foundation.example/node', 'Node.js license is MIT.')], rawCount: 2, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: new URL(url).hostname, title: 'Node.js',
      text: 'Node.js license is MIT.', retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(result.facts).toHaveLength(2);
  expect(result.evidence).toHaveLength(1);
  expect(result.evidence[0]).toMatchObject({ supportingSources: ['S1', 'S2'], assessment: 'corroborated',
    sourceDomains: ['nodejs.org', 'foundation.example'] });
  expect(result.verification?.allSupported).toBe(true);
});

it('does not increase confidence for repeated URLs or pages on the same domain', async () => {
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 2,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license\nNode.js licensing details' : request.systemPrompt.includes('fact extractor')
        ? [...request.userPrompt.matchAll(/(S\d+\.1):/g)].map(match => match[1]).join('\n') : '' }) },
    search: async () => ({ results: [hit('https://nodejs.org/about', 'Node.js license is MIT.'),
      hit('https://nodejs.org/license', 'Node.js license is MIT.')], rawCount: 2, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: 'nodejs.org', title: 'Node.js',
      text: 'Node.js license is MIT.', retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(result.facts).toHaveLength(2);
  expect(result.evidence[0]).toMatchObject({ sourceDomains: ['nodejs.org'], assessment: 'single_source' });
  expect(result.evidence[0].confidence).toBeLessThan(0.9);
});

it('cross-checks a snippet once, preserves freshness, and reports missing corroboration', async () => {
  const queries: Array<{ query: string; freshness: string }> = [];
  const result = await runStagedResearch({
    question: 'Latest Node.js version?', currentDate: '2026-10-08', freshness: 'month', maxQueries: 2,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js latest version 2026' : request.systemPrompt.includes('fact extractor') ? 'S1.1' : '' }) },
    search: async (query, freshness) => {
      queries.push({ query, freshness });
      return { results: [hit('https://nodejs.org/releases', 'Node.js version 24.1.0 is current.')], rawCount: 1, failureCount: 0, failures: [] };
    },
  });
  expect(queries).toHaveLength(2);
  expect(queries[1].query).not.toBe(queries[0].query);
  expect(queries[1].freshness).toBe('month');
  expect(result.facts).toHaveLength(1);
  expect(result.evidence[0].assessment).toBe('snippet_only');
  expect(result.limitations.join(' ')).toMatch(/snippet|corroborat/i);
  expect(result.answer).toMatch(/could not independently verify/i);
});

it('does not claim a generic fetched page verifies a fact found only in its snippet', async () => {
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license' : request.systemPrompt.includes('fact extractor') ? 'S1.1' : '' }) },
    search: async () => ({ results: [hit('https://nodejs.org/about', 'Node.js license is MIT.')], rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: 'nodejs.org', title: 'Node.js',
      text: 'Node.js license documentation and downloads.', retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(result.evidence[0].assessment).toBe('snippet_only');
  expect(result.evidence[0].confidence).toBeLessThan(0.6);
});

it('reports conflicting numeric variants without inventing a consensus', async () => {
  const queries: string[] = [];
  const result = await runStagedResearch({
    question: 'Node.js current version?', currentDate: '2026-10-08', maxQueries: 2,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js current version 2026' : request.systemPrompt.includes('fact extractor') ? 'S1.1\nS2.1' : 'Node.js version 24.1.0 is current. [S1]' }) },
    search: async query => {
      queries.push(query);
      return { results: [hit('https://one.example/releases', 'Node.js version 24.1.0 is current.'),
        hit('https://two.example/releases', 'Node.js version 24.2.0 is current.')], rawCount: 2, failureCount: 0, failures: [] };
    },
    fetchPage: async url => ({ id: url, url, domain: new URL(url).hostname, title: 'Node.js',
      text: `Node.js version ${url.includes('one.example') ? '24.1.0' : '24.2.0'} is current.`,
      retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(queries).toHaveLength(2);
  expect(result.evidence.every(claim => claim.status === 'conflicting')).toBe(true);
  expect(result.verification?.conflictingCount).toBe(2);
  expect(result.answer).toMatch(/sources disagree/i);
  expect(result.answer).toContain('24.1.0');
  expect(result.answer).toContain('24.2.0');
});

it('keeps different subjects and differently named benchmarks out of conflict detection', async () => {
  const result = await runStagedResearch({
    question: 'Alpha and Beta coding benchmark scores', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Alpha Beta coding benchmarks' : request.systemPrompt.includes('fact extractor') ? 'S1.1\nS2.1' : '' }) },
    search: async () => ({ results: [hit('https://one.example/alpha', 'Alpha scored 60% on Code-Bench coding.'),
      hit('https://two.example/beta', 'Beta scored 70% on Code-Bench coding.')], rawCount: 2, failureCount: 0, failures: [] }),
  });
  expect(result.evidence.every(claim => claim.status !== 'conflicting')).toBe(true);
});

it('reads independent pages concurrently with stable source ordering and no duplicate fetch', async () => {
  let active = 0;
  let peak = 0;
  const fetches: string[] = [];
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license' : request.systemPrompt.includes('fact extractor') ? 'S1.1\nS2.1\nS3.1' : '' }) },
    search: async () => ({ results: [hit('https://one.example/node', 'Node.js license is MIT.'),
      hit('https://two.example/node', 'Node.js license is MIT.'), hit('https://three.example/node', 'Node.js license is MIT.')],
      rawCount: 3, failureCount: 0, failures: [] }),
    fetchPage: async url => {
      fetches.push(url); active++; peak = Math.max(peak, active);
      await new Promise(resolve => setTimeout(resolve, url.includes('one.example') ? 15 : 1));
      active--;
      return { id: url, url, domain: new URL(url).hostname, title: 'Node.js', text: 'Node.js license is MIT.',
        retrievedAt: '2026-10-08', searchResultIds: [] };
    },
  });
  expect(peak).toBe(3);
  expect(fetches).toHaveLength(3);
  expect(Object.values(result.sources).map(source => source.url)).toEqual(fetches);
});

it('skips model synthesis when one exact page excerpt already answers the question', async () => {
  const synthesis = vi.fn();
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => {
      if (request.systemPrompt.includes('search queries')) return { text: 'Node.js license' };
      if (request.systemPrompt.includes('fact extractor')) return { text: 'S1.1' };
      synthesis(); return { text: '' };
    } },
    search: async () => ({ results: [hit('https://nodejs.org/about', 'Node.js license is MIT.')], rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: 'nodejs.org', title: 'Node.js',
      text: 'Node.js license is MIT.', retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(synthesis).not.toHaveBeenCalled();
  expect(result.answer).toContain('Node.js license is MIT');
});

it('uses its cross-check page budget on another domain ahead of another page from the original source', async () => {
  let searches = 0;
  const fetched: string[] = [];
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 2,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license' : request.systemPrompt.includes('fact extractor')
        ? [...request.userPrompt.matchAll(/(S\d+\.1):/g)].map(match => match[1]).join('\n') : '' }) },
    search: async () => ({ results: ++searches === 1 ? [hit('https://one.example/about', 'Node.js license is MIT.')]
      : [hit('https://one.example/license', 'Node.js license is MIT.'), hit('https://two.example/node', 'Node.js license is MIT.')],
      rawCount: searches === 1 ? 1 : 2, failureCount: 0, failures: [] }),
    fetchPage: async url => {
      fetched.push(url);
      return { id: url, url, domain: new URL(url).hostname, title: 'Node.js',
        text: url.endsWith('/about') ? 'Node.js license and downloads.' : 'Node.js license is MIT.',
        retrievedAt: '2026-10-08', searchResultIds: [] };
    },
  });
  expect(fetched).toEqual(['https://one.example/about', 'https://two.example/node']);
  expect(result.evidence[0].assessment).toBe('single_source'); // only the second domain supplied the page fact
});

it('does not treat publisher subdomains as independent corroboration', async () => {
  const result = await runStagedResearch({
    question: 'Node.js license?', currentDate: '2026-10-08', maxQueries: 1,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Node.js license' : request.systemPrompt.includes('fact extractor') ? 'S1.1\nS2.1' : '' }) },
    search: async () => ({ results: [hit('https://docs.nodejs.org/about', 'Node.js license is MIT.'),
      hit('https://blog.nodejs.org/about', 'Node.js license is MIT.')], rawCount: 2, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: new URL(url).hostname, title: 'Node.js',
      text: 'Node.js license is MIT.', retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  expect(result.evidence[0]).toMatchObject({ assessment: 'single_source', sourceDomains: ['nodejs.org'] });
  expect(result.evidence[0].confidence).toBeLessThan(0.9);
  expect(result.sources.S1.domain).toBe('docs.nodejs.org');
  expect(result.sources.S2.domain).toBe('blog.nodejs.org');
});

it('runs exactly the deep budget initial query count, not the old clamp of four', async () => {
  const searched: string[] = [];
  const model = { generate: async (request: GenerationRequest) => ({ text: request.systemPrompt.includes('search queries')
    ? 'Alpha performance\nBeta performance\nAlpha vs Beta\nAlpha accuracy\nBeta accuracy\nAlpha Beta comparison'
    : request.systemPrompt.includes('fact extractor') ? 'S1.1' : 'Alpha won. [S1]' }) };
  const planned = await new QueryPlanner(model).planQueries('Alpha vs Beta benchmarks', 'any', RESEARCH_BUDGETS.deep.initialQueries, '2026-10-08');
  expect(planned).toHaveLength(6);
  const result = await runStagedResearch({
    question: 'Alpha vs Beta benchmarks', currentDate: '2026-10-08',
    budget: RESEARCH_BUDGETS.deep,
    model,
    search: async query => { searched.push(query);
      return { results: [hit(`https://spec.example/${searched.length}`, `${query} result.`)], rawCount: 1, failureCount: 0, failures: [] }; },
  });
  // Every planned query was searched at least once; the old planner cap of
  // four would have silently dropped the last two.
  for (const item of planned) expect(searched).toContain(item.query);
  expect(result.queries.length).toBeGreaterThanOrEqual(6);
});

it('caps dynamically added follow-up queries at the budget followUpQueries', async () => {
  // Every search answers with a benchmark row naming only Alpha, so the
  // Beta gap keeps producing follow-ups until the budget stops them.
  const run = (followUpQueries: number) => runStagedResearch({
    question: 'Alpha vs Beta benchmarks', currentDate: '2026-10-08',
    budget: { ...RESEARCH_BUDGETS.normal, followUpQueries },
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Alpha benchmarks\nBeta benchmarks\nAlpha vs Beta benchmarks' : 'S1.1' }) },
    search: async query => ({ results: [hit(`https://table.example/${encodeURIComponent(query)}`, `${query} results.`)],
      rawCount: 1, failureCount: 0, failures: [] }),
    fetchPage: async url => ({ id: url, url, domain: 'table.example', title: 'Benchmarks',
      text: '| Benchmark | Alpha | Beta |\n| --- | --- | --- |\n| SWE-bench | 70% | — |',
      retrievedAt: '2026-10-08', searchResultIds: [] }),
  });
  const capped = await run(2);
  const disabled = await run(0);
  const distinct = (queries: { query: string }[]) => new Set(queries.map(q => q.query)).size;
  expect(distinct(capped.queries)).toBeLessThanOrEqual(distinct(disabled.queries) + 2);
  expect(distinct(disabled.queries)).toBe(3);
});

it('caps total page fetches at the budget pageFetches', async () => {
  const fetchPage = vi.fn(async (url: string) => ({ id: url, url, domain: new URL(url).hostname, title: 'Page',
    text: 'Alpha details and numbers.', retrievedAt: '2026-10-08', searchResultIds: [] }));
  const result = await runStagedResearch({
    question: 'Alpha specifications', currentDate: '2026-10-08',
    budget: { ...RESEARCH_BUDGETS.normal, pageFetches: 2 },
    maxQueries: 2,
    model: { generate: async request => ({ text: request.systemPrompt.includes('search queries')
      ? 'Alpha specs\nAlpha release' : 'S1.1' }) },
    search: async query => ({ results: [1, 2, 3].map(i => hit(`https://${query.includes('release') ? 'r' : 's'}${i}.example/page`, `${query} result ${i}.`, i)),
      rawCount: 3, failureCount: 0, failures: [] }),
    fetchPage,
  });
  expect(fetchPage.mock.calls.length).toBeLessThanOrEqual(2);
  expect(result.pagesFetched).toBeLessThanOrEqual(2);
});
