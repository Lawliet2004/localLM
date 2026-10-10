import { expect, it } from 'vitest';
import { planAgentSearch } from './agent_search';
import { comparisonSubjects } from './fallback_planner';

it('distinguishes comparison subjects from a trailing for-topic clause', () => {
  expect(comparisonSubjects('Compare Alpha and Beta for coding and knowledge work benchmarks')).toEqual(['Alpha', 'Beta']);
  expect(comparisonSubjects('Compare current coding benchmarks for Qwen and DeepSeek')).toEqual(['Qwen', 'DeepSeek']);
});

it('decomposes a full AREX comparison question into short searches', async () => {
  const result = await planAgentSearch({
    queries: ['Compare coding benchmarks between Llama 3 and Mistral 7B?'],
    currentDate: '2026-09-28',
  });
  expect(result.queries).toEqual(['Llama 3 coding benchmarks', 'Mistral 7B coding benchmarks', 'Llama 3 vs Mistral 7B coding benchmarks']);
});

it('injects the year into a direct current-version search', async () => {
  const result = await planAgentSearch({ queries: ['What is the latest Node.js version?'], currentDate: '2026-09-28' });
  expect(result.queries).toEqual(['Node.js latest version 2026']);
});

it('asks for referents when an AREX search names neither model', async () => {
  const result = await planAgentSearch({
    queries: ['Compare benchmarks between these two models'], currentDate: '2026-09-28',
    model: { generate: async () => ({ text: 'NEEDS_CLARIFICATION' }) },
  });
  expect(result.queries).toEqual([]);
  expect(result.clarificationQuestion).toMatch(/which two models/i);
});

it('resolves AREX comparison references from recent conversation text', async () => {
  const result = await planAgentSearch({
    queries: ['Compare benchmarks between these two models'],
    context: 'The models were Llama 3 and Mistral 7B.', currentDate: '2026-09-28',
    model: { generate: async request => ({ text: request.systemPrompt.includes('reference resolver')
      ? 'Llama 3\nMistral 7B' : 'invalid query output' }) },
  });
  expect(result.queries).toContain('Llama 3 benchmarks');
  expect(result.queries).toContain('Mistral 7B benchmarks');
});
it('preserves explicit operators and exact phrases', async () => {
  const query = 'site:example.org "adapter charging" -obsolete filetype:pdf intitle:spec';
  expect((await planAgentSearch({ queries: [query] })).queries).toEqual([query]);
});
