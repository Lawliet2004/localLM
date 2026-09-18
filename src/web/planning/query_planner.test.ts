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
