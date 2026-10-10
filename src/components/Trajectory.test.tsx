import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Trajectory } from './Trajectory';

vi.mock('../lib/api', () => ({
  nativeAvailable: false,
  errorMessage: (e: unknown) => String(e),
  api: { getSessionEvents: vi.fn(async () => []), forkSession: vi.fn() },
}));

describe('trajectory category tabs', () => {
  it('exposes real tabs with selection state and arrow-key navigation', async () => {
    render(<Trajectory conversationId="c1" />);
    const list = await screen.findByRole('tablist', { name: 'Event category filters' });
    const tabs = within(list).getAllByRole('tab');
    expect(tabs.map(tab => tab.textContent)).toEqual(['All', 'Model', 'Tool', 'System', 'Error']);
    expect(within(list).getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
    expect(within(list).getByRole('tab', { name: 'All' })).toHaveAttribute('tabIndex', '0');
    expect(within(list).getByRole('tab', { name: 'Model' })).toHaveAttribute('tabIndex', '-1');

    fireEvent.keyDown(list, { key: 'ArrowRight' });
    expect(within(list).getByRole('tab', { name: 'Model' })).toHaveAttribute('aria-selected', 'true');
    expect(within(list).getByRole('tab', { name: 'Model' })).toHaveAttribute('tabIndex', '0');
    expect(within(list).getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'false');
    await waitFor(() => expect(within(list).getByRole('tab', { name: 'Model' })).toHaveFocus());

    fireEvent.keyDown(list, { key: 'ArrowLeft' });
    expect(within(list).getByRole('tab', { name: 'All' })).toHaveAttribute('aria-selected', 'true');
    await waitFor(() => expect(within(list).getByRole('tab', { name: 'All' })).toHaveFocus());
  });
});
