import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { SessionTabs, readOpenSessions } from './SessionTabs';
import { emptySessionRun } from '../hooks/useSessionRuns';
const conversations = [{ id: 'a', title: 'Research', updatedAt: 1, providerId: null, modelId: null, providerSelectionRequired: false }, { id: 'b', title: 'Document', updatedAt: 2, providerId: null, modelId: null, providerSelectionRequired: false }];
afterEach(() => localStorage.removeItem('locallm-open-sessions'));
it('switches with arrows/Home/End and closes a tab without a stop operation', () => {
  const select = vi.fn(), close = vi.fn();
  render(<SessionTabs ids={['a', 'missing', 'b']} conversations={conversations} activeId="a" runs={{ a: { ...emptySessionRun(), generating: true } }} disabled={false} onSelect={select} onClose={close} onNew={vi.fn()} />);
  const tabs = screen.getAllByRole('tab'); expect(tabs).toHaveLength(2);
  tabs[0].focus(); fireEvent.keyDown(tabs[0], { key: 'ArrowRight' }); expect(tabs[1]).toHaveFocus(); expect(select).toHaveBeenLastCalledWith('b');
  fireEvent.keyDown(tabs[1], { key: 'Home' }); expect(tabs[0]).toHaveFocus();
  fireEvent.keyDown(tabs[0], { key: 'End' }); expect(select).toHaveBeenLastCalledWith('b');
  fireEvent.click(screen.getByRole('button', { name: 'Close tab Research' })); expect(close).toHaveBeenCalledWith('a');
});
it('rejects malformed saved tabs and deduplicates IDs', () => {
  localStorage.setItem('locallm-open-sessions', '{broken'); expect(readOpenSessions()).toEqual([]);
  localStorage.setItem('locallm-open-sessions', JSON.stringify(['a', null, {}, 'b', 'a', ''])); expect(readOpenSessions()).toEqual(['a', 'b']);
});
