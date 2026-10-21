import { expect, it } from 'vitest';
import { QueryPlanner } from './query_planner';
import type { GenerationRequest } from '../types';

it('constrains the planner to its configured query count', async () => {
  let request: GenerationRequest | undefined;
  const planner = new QueryPlanner({ generate: async value => {
    request = value;
    return { text: '{"queries":[{"query":"React release","purpose":"release"}]}' };
  }});
  await planner.planQueries('Latest React?', 'month', 2);
  expect(request?.responseSchema).toMatchObject({ properties: { queries: { maxItems: 2 } } });
});

it('uses the original question unchanged when model JSON is invalid', async () => {
  const question = 'What changed in Node.js v22.12.0?';
  const planner = new QueryPlanner({ generate: async () => ({ text: 'not JSON' }) });
  expect((await planner.planQueries(question)).map(q => q.query)).toEqual([question]);
});

it('rules strategy never calls the model', async () => {
  let called = false;
  const planner = new QueryPlanner({ generate: async () => {
    called = true;
    return { text: '{"queries":[{"query":"should not be used"}]}' };
  }});
  const queries = await planner.planQueries('Compare A and B', 'any', 3, undefined, 'rules');
  expect(called).toBe(false);
  expect(queries[0].query).toBe('Compare A and B');
});

it('model planning requests low reasoning effort', async () => {
  let request: GenerationRequest | undefined;
  const planner = new QueryPlanner({ generate: async value => {
    request = value;
    return { text: '{"queries":[{"query":"q","purpose":"p"}]}' };
  }});
  await planner.planQueries('Latest React?', 'month', 2);
  expect(request?.reasoningEffort).toBe('low');
});
