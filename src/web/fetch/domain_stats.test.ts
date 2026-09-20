import { expect, it } from 'vitest';
import { InMemoryStorageAdapter } from '../cache/sqlite';
import { DomainStatsStore, orderByDomainHealth } from './domain_stats';

it('marks a domain chronic after repeated failures in the current window', async () => {
  const store = new DomainStatsStore(new InMemoryStorageAdapter(), { minAttempts: 4, chronicFailRate: 0.7 });
  for (let i = 0; i < 4; i++) await store.record('blocked.example', false, 'HTTP 403');
  const status = await store.status('blocked.example');
  expect(status.chronic).toBe(true);
  expect(status.fail).toBe(4);
});

it('does not treat a cold-start domain as chronic', async () => {
  const store = new DomainStatsStore(new InMemoryStorageAdapter());
  await store.record('fresh.example', false, 'timeout');
  expect((await store.status('fresh.example')).chronic).toBe(false);
});

it('moves chronic domains behind healthy ones', async () => {
  const store = new DomainStatsStore(new InMemoryStorageAdapter(), { minAttempts: 4, chronicFailRate: 0.7 });
  for (let i = 0; i < 4; i++) await store.record('bad.example', false);
  const { ordered, deprioritized } = await orderByDomainHealth([
    { id: '1', queryId: 'q', title: 'bad', url: 'https://bad.example/a', domain: 'bad.example', rank: 1 },
    { id: '2', queryId: 'q', title: 'ok', url: 'https://ok.example/a', domain: 'ok.example', rank: 2 },
  ], store);
  expect(ordered.map((r) => r.domain)).toEqual(['ok.example', 'bad.example']);
  expect(deprioritized).toEqual(['bad.example']);
});
