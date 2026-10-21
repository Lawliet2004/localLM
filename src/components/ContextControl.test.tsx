import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ContextControl } from './ContextControl';
const mocks = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  compactionStatus: async () => ({ auto: true }), setCompactionAuto: mocks.save,
} }));
it('shows reserved context and persists opting out without optimistic state loss on failure', async () => {
  mocks.save.mockRejectedValueOnce('Database unavailable').mockResolvedValueOnce(undefined);
  render(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 700, responseReserve: 100, contextLength: 1000 }} />);
  fireEvent.click(screen.getByRole('button', { name: /Context 80% used/ }));
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
