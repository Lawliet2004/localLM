import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { TabDock } from './TabDock';

function renderDock(page: Parameters<typeof TabDock>[0]['page'] = 'chat') {
  const onPage = vi.fn();
  const onTheme = vi.fn();
  render(<TabDock page={page} onPage={onPage} theme="light" onTheme={onTheme} />);
  return { onPage, onTheme };
}

describe('TabDock', () => {
  it('exposes the app sections as a labelled floating navigation', () => {
    renderDock();
    const nav = screen.getByRole('navigation', { name: 'Main navigation' });
    expect(nav).toHaveClass('app-dock');
    for (const name of ['LocalLM home', 'Conversations', 'Models & runtime', 'Execution', 'Connectors', 'Skills', 'Tools', 'Use dark theme', 'Open settings']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
  });

  it('marks the current page and navigates on click', async () => {
    const { onPage } = renderDock('connectors');
    expect(screen.getByRole('button', { name: 'Connectors' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('button', { name: 'Conversations' })).not.toHaveAttribute('aria-current');
    await userEvent.click(screen.getByRole('button', { name: 'Skills' }));
    expect(onPage).toHaveBeenCalledWith('skills');
    await userEvent.click(screen.getByRole('button', { name: 'LocalLM home' }));
    expect(onPage).toHaveBeenCalledWith('chat');
  });

  it('marks settings active for any settings page and toggles the theme', async () => {
    const { onPage, onTheme } = renderDock('appearance');
    const settings = screen.getByRole('button', { name: 'Open settings' });
    expect(settings).toHaveAttribute('aria-current', 'page');
    await userEvent.click(settings);
    expect(onPage).toHaveBeenCalledWith('general');
    await userEvent.click(screen.getByRole('button', { name: 'Use dark theme' }));
    expect(onTheme).toHaveBeenCalled();
  });
});
