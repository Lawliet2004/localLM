import { act, render } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message } from '../lib/types';
const parsed = vi.hoisted(() => new Map<string, number>());
vi.mock('react-markdown', async importOriginal => {
  const original = await importOriginal<typeof import('react-markdown')>();
  const React = await import('react');
  return { ...original, default: (props: Parameters<typeof original.default>[0]) => {
    const text = String(props.children); parsed.set(text, (parsed.get(text) || 0) + 1);
    return React.createElement(original.default, props);
  } };
});
vi.mock('../lib/api', () => ({ nativeAvailable: false, errorMessage: String, api: {} }));
it('keeps completed Markdown responses out of subsequent streaming renders', async () => {
  const history: Message[] = Array.from({ length: 16 }, (_, index) => [
    { id: `u${index}`, role: 'user' as const, content: `Question ${index}` },
    { id: `a${index}`, role: 'assistant' as const, content: `Completed **response ${index}**` },
  ]).flat().map(row => ({ ...row, conversationId: 'fixture', reasoning: '', status: 'complete', createdAt: 0 }));
  const last: Message = { id: 'stream', conversationId: 'fixture', role: 'assistant', content: 'Streaming', reasoning: '', status: 'streaming', createdAt: 0 };
  const props = { generating: true, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
  const view = render(<Chat {...props} messages={[...history, last]} />);
  await act(async () => {});
  const before = history.filter(row => row.role === 'assistant').map(row => parsed.get(row.content) || 0);
  for (let index = 0; index < 5; index++) view.rerender(<Chat {...props} messages={[...history, { ...last, content: `Streaming ${index}` }]} />);
  const additional = history.filter(row => row.role === 'assistant').reduce((total, row, index) => total + (parsed.get(row.content) || 0) - before[index], 0);
  expect(additional, 'completed responses reparsed during five stream updates').toBe(0);
});
