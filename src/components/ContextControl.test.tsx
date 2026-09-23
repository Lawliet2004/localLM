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
