import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ContextControl } from './ContextControl';
const mocks = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  compactionStatus: async () => ({ auto: true }), setCompactionAuto: mocks.save,
} }));
it('shows input usage separately from the response reserve and persists opting out without optimistic state loss on failure', async () => {
  mocks.save.mockRejectedValueOnce('Database unavailable').mockResolvedValueOnce(undefined);
  render(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 700, responseReserve: 100, contextLength: 1000 }} />);
  fireEvent.click(screen.getByRole('button', { name: /Context 70% used/ }));
  const checkbox = screen.getByRole('checkbox', { name: 'Auto-compact at 80%' });
  expect(checkbox).toBeChecked();
  fireEvent.click(checkbox);
  await screen.findByRole('alert');
  expect(checkbox).toBeChecked();
  fireEvent.click(checkbox);
  await waitFor(() => expect(checkbox).not.toBeChecked());
  expect(mocks.save).toHaveBeenLastCalledWith('chat', false);
});

it('shows measured cache reuse and prompt processing time', () => {
  render(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 1000, responseReserve: 100, contextLength: 4096, cachedTokens: 800, cacheHitRate: 0.8, cacheWriteTokens: 100, promptMs: 250 }} />);
  const cache = screen.getByLabelText('Prompt cache usage');
  expect(cache).toHaveTextContent('800 input tokens reused (80% cache hit)');
  expect(cache).toHaveTextContent('100 written to cache');
  expect(cache).toHaveTextContent('0.25s prompt processing');
});

it('keeps missing cache metrics unknown and displays a measured cache miss', () => {
  const { rerender } = render(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 1000, responseReserve: 100, contextLength: 4096 }} />);
  expect(screen.queryByLabelText('Prompt cache usage')).not.toBeInTheDocument();
  rerender(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 1000, responseReserve: 100, contextLength: 4096, cachedTokens: 0, cacheHitRate: 0 }} />);
  expect(screen.getByLabelText('Prompt cache usage')).toHaveTextContent('0 input tokens reused (0% cache hit)');
});


it('isolates pending compaction updates and popover state when switching chats', async () => {
  let finish!: () => void;
  mocks.save.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
  const { rerender } = render(<ContextControl conversationId="first" busy={false} />);
  fireEvent.click(screen.getByRole('button', { name: 'Context usage details' }));
  fireEvent.click(screen.getByRole('checkbox'));
  expect(screen.getByRole('checkbox')).toBeDisabled();

  rerender(<ContextControl conversationId="second" busy={false} />);
  await waitFor(() => expect(screen.getByRole('checkbox')).toBeEnabled());
  expect(screen.getByRole('button', { name: 'Context usage details' })).toHaveAttribute('aria-expanded', 'false');
  expect(screen.getByRole('checkbox')).toBeChecked();
  await act(async () => finish());
  expect(screen.getByRole('checkbox')).toBeChecked();
  expect(mocks.save).toHaveBeenLastCalledWith('first', false);
});

it('does not count a million-context model output allowance as used history', () => {
  const { rerender } = render(<ContextControl conversationId="first" busy={false} usage={{ inputTokens: 4398, responseReserve: 393216, contextLength: 1048576, estimated: true }} />);
  expect(screen.getByRole('button', { name: /Context 0% used/ })).toBeInTheDocument();
  expect(screen.getByText('4,398 / 1,048,576 tokens')).toBeInTheDocument();
  expect(screen.getByText(/Estimated: 4,398 input/)).toHaveTextContent('393,216 reserved');
  rerender(<ContextControl conversationId="fresh" busy={false} />);
  expect(screen.getByText('No usage measured yet')).toBeInTheDocument();
  expect(screen.queryByText('4,398 / 1,048,576 tokens')).not.toBeInTheDocument();
  rerender(<ContextControl conversationId="first" busy={false} usage={{ inputTokens: 4398, responseReserve: 393216, contextLength: 1048576 }} />);
  expect(screen.getByText('4,398 / 1,048,576 tokens')).toBeInTheDocument();
});
