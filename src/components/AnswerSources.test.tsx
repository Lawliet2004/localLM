import { expect, it } from 'vitest';
import { collectAnswerSources } from './AnswerSources';
import type { Message } from '../lib/types';
it('deduplicates stored sources while preserving citation ids and supplied metadata', () => {
  const message = (sources: unknown): Message => ({ id: 't', conversationId: 'c', role: 'tool', content: JSON.stringify({ result: { sources } }), reasoning: '', status: 'complete', createdAt: 1 });
  const sources = collectAnswerSources([message([{ id: 'a', url: 'https://example.org/page#one', title: 'Evidence', snippet: 'Actual snippet' }, { url: 'javascript:alert(1)' }]), message({ b: { url: 'https://example.org/page#two', date: '2026-10-17' } })]);
  expect(sources).toHaveLength(1);
  expect(sources[0]).toMatchObject({ title: 'Evidence', snippet: 'Actual snippet', date: '2026-10-17', ids: ['a', 'b'] });
});
