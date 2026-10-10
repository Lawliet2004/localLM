import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ContextControl } from './ContextControl';
const mocks = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  compactionStatus: async () => ({ auto: true }), setCompactionAuto: mocks.save,
} }));
it('shows reserved context and persists opting out without optimistic state loss on failure', async () => {
  mocks.save.mockRejectedValueOnce('Database unavailable').mockResolvedValueOnce(undefined);
  render(<ContextControl conversationId="chat" busy={false} usage={{ inputTokens: 700, outputTokens: 42, cachedInputTokens: 250, responseReserve: 100, contextLength: 1000 }} />);
  expect(screen.queryByRole('dialog', { name: 'Context usage and compaction' })).not.toBeInTheDocument();
  fireEvent.mouseEnter(screen.getByRole('button', { name: /Context 80% used/ }));
  const panel = screen.getByRole('dialog', { name: 'Context usage and compaction' });
  expect(within(panel).getByText('Input tokens').nextElementSibling).toHaveTextContent('700');
  expect(within(panel).getByText('Output tokens').nextElementSibling).toHaveTextContent('42');
  expect(within(panel).getByText('Cached input').nextElementSibling).toHaveTextContent('250');
  expect(within(panel).getByText('Reply reserve').nextElementSibling).toHaveTextContent('100');
  const checkbox = screen.getByRole('checkbox', { name: 'Auto-compact at 80%' });
  expect(screen.getByText(/Cached input is included in input tokens/)).toBeVisible();
  expect(checkbox).toBeChecked();
  fireEvent.click(checkbox);
  await screen.findByRole('alert');
  expect(checkbox).toBeChecked();
  fireEvent.click(checkbox);
  await waitFor(() => expect(checkbox).not.toBeChecked());
  expect(mocks.save).toHaveBeenLastCalledWith('chat', false);
  fireEvent.mouseLeave(panel);
  expect(screen.queryByRole('dialog', { name: 'Context usage and compaction' })).not.toBeInTheDocument();
});

it('shows unavailable output and cache counts without guessing', () => {
  render(<ContextControl conversationId="new" busy={false} usage={{ inputTokens: 20, responseReserve: 50, contextLength: 1000 }} />);
  fireEvent.mouseEnter(screen.getByRole('button', { name: /Context 7% used/ }));
  expect(screen.getAllByText('Unavailable')).toHaveLength(2);
});
