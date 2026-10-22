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

it('follows asynchronous size changes only at the bottom, and pauses for disclosure inspection', async () => {
  const callbacks: (() => void)[] = [];
  vi.stubGlobal('ResizeObserver', class { constructor(callback: () => void) { callbacks.push(callback); } observe() {} disconnect() {} });
  try {
    const history: Message[] = [{ id: 'tool', conversationId: 'fixture', role: 'tool', content: JSON.stringify({ request: { name: 'execute_command', arguments: { command: 'test' } }, result: { stdout: 'done' } }), reasoning: '', status: 'complete', createdAt: 0 }];
    const props = { generating: false, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
    const { container } = render(<Chat {...props} conversationKey="fixture" messages={history} />);
    const viewport = container.querySelector('.chat-scroll') as HTMLDivElement;
    Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 2000 });
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 400 });
    await act(async () => { callbacks[callbacks.length - 1](); });
    expect(viewport.scrollTop).toBe(2000);
    viewport.scrollTop = 100;
    const { fireEvent } = await import('@testing-library/react');
    fireEvent.scroll(viewport);
    await act(async () => { callbacks[callbacks.length - 1](); });
    expect(viewport.scrollTop).toBe(100);
    viewport.scrollTop = 1600; fireEvent.scroll(viewport);
    fireEvent.click(container.querySelector('.command-run-card > summary')!);
    await act(async () => { callbacks[callbacks.length - 1](); });
    expect(viewport.scrollTop).toBe(1600);
  } finally { vi.unstubAllGlobals(); }
});

it('does not reparse completed prose blocks within a growing answer', async () => {
  const { MarkdownView } = await import('./MarkdownView');
  const prefix = Array.from({ length: 40 }, (_, i) => `Stable paragraph ${i}.\n\n`).join('');
  const view = render(<MarkdownView content={`${prefix}Current words`} streaming />);
  const before = parsed.get('Stable paragraph 0.\n\n') ?? 0;
  expect(before).toBeGreaterThan(0);
  for (let i = 0; i < 10; i++) view.rerender(<MarkdownView content={`${prefix}Current words ${i}`} streaming />);
  expect(parsed.get('Stable paragraph 0.\n\n')).toBe(before);
});
