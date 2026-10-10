import { act, render } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message } from '../lib/types';

const fixture = vi.hoisted(() => ({ markdown: vi.fn() }));
vi.mock('react-markdown', () => ({ default: ({ children }: { children?: string }) => {
  fixture.markdown(children);
  return <p>{children}</p>;
} }));

it('does not render completed turn markdown again while a later answer streams', () => {
  const history: Message[] = Array.from({ length: 200 }, (_, i): Message[] => [
    { id: `user-${i}`, conversationId: 'chat', role: 'user', content: `Question ${i}`, reasoning: '', status: 'complete', createdAt: i * 2 },
    { id: `answer-${i}`, conversationId: 'chat', role: 'assistant', content: `Answer ${i}`, reasoning: '', status: 'complete', createdAt: i * 2 + 1 },
  ]).flat();
  const user: Message = { ...history[0], id: 'live-user', content: 'Next question', createdAt: 401 };
  const live: Message = { ...history[1], id: 'live-answer', content: 'Streaming', status: 'streaming', createdAt: 402 };
  const props = { generating: true, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
  fixture.markdown.mockClear();
  const view = render(<Chat {...props} messages={[...history, user, live]} />);
  const initialOldRenders = fixture.markdown.mock.calls.filter(([text]) => text === 'Answer 50').length;
  expect(initialOldRenders).toBeGreaterThan(0);
  for (let chunk = 0; chunk < 20; chunk++) view.rerender(<Chat {...props} messages={[...history, user, { ...live, content: `Streaming ${chunk}` }]} />);
  expect(fixture.markdown.mock.calls.filter(([text]) => text === 'Answer 50')).toHaveLength(initialOldRenders);
  expect(fixture.markdown.mock.calls.filter(([text]) => text === 'Streaming 19')).toHaveLength(1);
});

it('does not reparse an idle stream every 100ms when there is no reasoning', async () => {
  vi.useFakeTimers();
  try {
    const message: Message = { id: 'live', conversationId: 'chat', role: 'assistant', content: 'Current answer', reasoning: '', status: 'streaming', createdAt: 1 };
    const view = render(<Chat generating ready loading={false} messages={[message]} onSend={vi.fn()} onCancel={vi.fn()} onConfigure={vi.fn()} />);
    const initialRenders = fixture.markdown.mock.calls.length;
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(fixture.markdown).toHaveBeenCalledTimes(initialRenders);
    view.unmount();
  } finally { vi.useRealTimers(); }
});
