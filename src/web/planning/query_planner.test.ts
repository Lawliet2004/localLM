import { expect, it } from 'vitest';
import { QueryPlanner } from './query_planner';
import type { GenerationRequest } from '../types';

it('accepts short keyword lines from a small model without padding them', async () => {
  const planner = new QueryPlanner({ generate: async () => ({ text: '<think>Find each part.</think>\n- Node.js license\n- Node.js downloads' }) });
  const queries = await planner.planQueries('Find the license and downloads for Node.js');
  expect(queries.map(q => q.query)).toEqual(['Node.js license', 'Node.js downloads']);
});

it('keeps a single specific keyword usable', async () => {
  const planner = new QueryPlanner({ generate: async () => ({ text: 'SQLite' }) });
  expect((await planner.planQueries('Tell me about SQLite')).map(q => q.query)).toEqual(['SQLite']);
});

it('searches explicit question parts separately when planning fails', async () => {
  const planner = new QueryPlanner({ generate: async () => { throw new Error('model unavailable'); } });
  const queries = await planner.planQueries('What is the Node.js license? Where can I download SQLite?');
  expect(queries.map(q => q.query)).toEqual(['Node.js license', 'download SQLite']);
});

it('asks the model for short line queries with the current date', async () => {
  let request: GenerationRequest | undefined;
  const planner = new QueryPlanner({ generate: async value => {
    request = value;
    return { text: 'Node.js latest version\nNode.js official releases' };
  }});
  const queries = await planner.planQueries('What is the latest version of Node.js?', 'month', 2, '2026-09-27');
  expect(request?.temperature).toBe(0);
  expect(request?.responseSchema).toBeUndefined();
  expect(request?.systemPrompt).toContain('2026-09-27');
  expect(queries.map(q => q.query)).toEqual(['Node.js latest version 2026', 'Node.js official releases 2026']);
});

it('rejects sentence queries and decomposes a comparison when model output is invalid', async () => {
  const planner = new QueryPlanner({ generate: async () => ({ text: 'Can you compare benchmarks between GPT-6 Astra and Opus 5.5?' }) });
  const queries = await planner.planQueries('Compare coding benchmarks between GPT-6 Astra and Opus 5.5', 'month', 4, '2026-09-27');
  expect(queries.map(q => q.query)).toEqual([
    'GPT-6 Astra coding benchmarks',
    'Opus 5.5 coding benchmarks',
    'GPT-6 Astra vs Opus 5.5 coding benchmarks',
  ]);
});

it('never falls back to searching the full user question', async () => {
  const question = 'What changed in the latest Node.js version?';
  const planner = new QueryPlanner({ generate: async () => ({ text: 'not a query' }) });
  const queries = await planner.planQueries(question, 'month', 4, '2026-09-27');
  expect(queries[0].query).toBe('Node.js latest version 2026');
  expect(queries.every(q => q.query !== question)).toBe(true);
});

it('splits a direct versus comparison into entity and comparison searches', async () => {
  const queries = await new QueryPlanner().planQueries('Llama 3 vs Mistral 7B benchmark scores', 'any', 4, '2026-09-27');
  expect(queries.map(q => q.query)).toEqual([
    'Llama 3 benchmarks',
    'Mistral 7B benchmarks',
    'Llama 3 vs Mistral 7B benchmarks',
  ]);
});

it('retains the current year when a generated query already has eight words', async () => {
  const planner = new QueryPlanner({ generate: async () => ({ text: 'Node.js latest stable version release notes package manager' }) });
  const queries = await planner.planQueries('Latest Node.js version?', 'month', 1, '2026-09-27');
  expect(queries[0].query.split(' ')).toHaveLength(8);
  expect(queries[0].query.endsWith('2026')).toBe(true);
});
