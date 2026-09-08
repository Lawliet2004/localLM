import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message } from '../lib/types';

const props = { generating: false, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
describe('chat action reporting and submission', () => {
  it('labels measured context separately from draft changes', () => {
    render(<Chat {...props} messages={[]} contextUsage={{ inputTokens: 123, responseReserve: 512, contextLength: 8192 }} />);
    const usage = screen.getByLabelText('Last request context');
    expect(usage).toHaveTextContent('123 input + 512 response reserve');
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'A new draft that has not been counted' } });
    expect(usage).toHaveTextContent('123 input');
    expect(usage).toHaveTextContent('Draft changes are not included');
  });
  it.each([
    ['complete', { isError: true, message: 'Process failed' }, 'allowed', 'Failed'],
    ['complete', { isError: true }, 'denied', 'Denied'],
    ['interrupted', 'Cancelled', 'allowed', 'Stopped · outcome unknown'],
    ['complete', { isError: false }, 'allowed', 'Finished'],
  ] as const)('reports %s tool outcomes accurately (%s)', (status, result, decision, label) => {
    const message: Message = { id: 'tool', conversationId: 'chat', role: 'tool', reasoning: '', createdAt: 0, status,
      content: JSON.stringify({ request: { connector: 'Local', name: 'run_code', decision }, result }) };
    render(<Chat {...props} messages={[message]} />);
    expect(screen.getByText(label)).toBeInTheDocument();
  });
  it('blocks duplicate submission and preserves a new draft after rejection', async () => {
    let reject!: (error: Error) => void;
    const onSend = vi.fn(() => new Promise<void>((_, fail) => { reject = fail; }));
    render(<Chat {...props} messages={[]} onSend={onSend} />);
    const input = screen.getByLabelText('Message');
    fireEvent.change(input, { target: { value: 'First request' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.change(input, { target: { value: 'Next draft' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSend).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error('Rejected')));
    expect(input).toHaveValue('Next draft');
  });
});

it('shows a saved generation error next to the partial response', () => {
  const message: Message = { id: 'failed', conversationId: 'chat', role: 'assistant', content: 'Partial result', reasoning: '', status: 'error', createdAt: 0, error: 'Response token limit reached. Increase the response limit.' };
  render(<Chat {...props} messages={[message]} />);
  expect(screen.getByText('Partial result')).toBeInTheDocument();
  expect(screen.getByText('Response token limit reached. Increase the response limit.')).toBeInTheDocument();
});
