import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { WorkspacePanel } from './WorkspacePanel';

const mocks = vi.hoisted(() => ({
  getWorkspace: vi.fn(),
  workspaceGit: vi.fn(),
  setWorkspace: vi.fn(),
  open: vi.fn(),
  openPath: vi.fn(),
  closeTerminalForTab: vi.fn(),
  terminalMounts: [] as string[],
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    getWorkspace: mocks.getWorkspace,
    workspaceGit: mocks.workspaceGit,
    setWorkspace: mocks.setWorkspace,
  },
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: mocks.openPath }));
// The real panels drive PTY/native-webview calls; the tab shell is what this
// suite covers.
vi.mock('./TerminalPanel', async () => {
  const { useEffect } = await import('react');
  return {
    // Recording real mounts proves the keyed remount per terminal tab.
    TerminalPanel: ({ tabKey }: { tabKey: string }) => {
      useEffect(() => { mocks.terminalMounts.push(tabKey); }, []);
      return <div data-testid="terminal-panel">{tabKey}</div>;
    },
    closeTerminalForTab: mocks.closeTerminalForTab,
  };
});
vi.mock('./BrowserPanel', () => ({ BrowserPanel: () => <div data-testid="browser-panel" /> }));
vi.mock('./FilesPanel', () => ({
  WorkspaceTree: () => <div data-testid="workspace-tree" />,
  FileView: () => null,
}));

describe('WorkspacePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.terminalMounts.length = 0;
    mocks.getWorkspace.mockResolvedValue({ path: 'C:/workspace' });
    mocks.workspaceGit.mockResolvedValue({ branch: 'main', branches: ['main', 'dev'], status: '', diff: '' });
  });

  const renderPanel = () => render(<WorkspacePanel busy={false} revision="proj:conv-1" />);

  it('keeps the toggle bar inside the minimal-header-exempt container', () => {
    const { container } = renderPanel();
    expect(container.querySelector('.workspace-panel-container > .panel-toggles')).not.toBeNull();
    expect(screen.getByRole('button', { name: 'Toggle workspace panel' })).toBeInTheDocument();
  });

  it('opens the launcher and creates tabs for every feature', async () => {
    const user = userEvent.setup();
    renderPanel();
    await user.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
    const launcher = document.querySelector<HTMLElement>('nav.workspace-empty')!;
    expect(launcher.getAttribute('aria-label')).toBe('Workspace shortcuts');
    for (const label of ['Review', 'Terminal', 'Browser', 'Files']) {
      expect(within(launcher).getByRole('button', { name: new RegExp(`^${label}`) })).toBeInTheDocument();
    }
    await user.click(within(launcher).getByRole('button', { name: /^Files/ }));
    expect(await screen.findByTestId('workspace-tree')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Open file/ })).toBeInTheDocument();
    // The + menu launches a second feature tab while the first stays open.
    await user.click(document.querySelector<HTMLElement>('.workspace-add summary')!);
    await user.click(within(document.querySelector<HTMLElement>('.workspace-add nav')!).getByRole('button', { name: /^Browser/ }));
    expect(await screen.findByTestId('browser-panel')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Open file/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^New tab/ })).toBeInTheDocument();
  });

  it('supports keyboard shortcuts, expand, tab close, and closes on Escape', async () => {
    const user = userEvent.setup();
    renderPanel();
    fireEvent.keyDown(window, { key: '`', ctrlKey: true });
    expect(await screen.findByTestId('terminal-panel')).toBeInTheDocument();
    await user.click(screen.getAllByRole('button', { name: 'Expand panel' })[0]);
    expect(document.querySelector('.workspace-panel.panel-expanded')).not.toBeNull();
    await user.click(screen.getByRole('button', { name: 'Close Terminal' }));
    expect(screen.queryByTestId('terminal-panel')).not.toBeInTheDocument();
    expect(mocks.closeTerminalForTab).toHaveBeenCalled();
    fireEvent.keyDown(window, { key: 'G', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(mocks.workspaceGit).toHaveBeenCalled());
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('complementary', { name: 'Workspace inspector' })).not.toBeInTheDocument();
  });

  it('mounts a distinct TerminalPanel per terminal tab and kills that tab\'s session on close', async () => {
    const user = userEvent.setup();
    renderPanel();
    await user.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
    await user.click(within(document.querySelector<HTMLElement>('nav.workspace-empty')!).getByRole('button', { name: /^Terminal/ }));
    await waitFor(() => expect(mocks.terminalMounts).toHaveLength(1));
    const firstKey = mocks.terminalMounts[0];
    // The + menu spawns a second terminal tab alongside the first.
    await user.click(document.querySelector<HTMLElement>('.workspace-add summary')!);
    await user.click(within(document.querySelector<HTMLElement>('.workspace-add nav')!).getByRole('button', { name: /^Terminal/ }));
    await waitFor(() => expect(mocks.terminalMounts).toHaveLength(2));
    const secondKey = mocks.terminalMounts[1];
    expect(screen.getByTestId('terminal-panel')).toHaveTextContent(secondKey);
    // Switching back remounts a panel bound to the first tab's session.
    fireEvent.click(document.querySelectorAll<HTMLElement>('.workspace-tab')[0]);
    await waitFor(() => expect(mocks.terminalMounts).toHaveLength(3));
    expect(mocks.terminalMounts[2]).toBe(firstKey);
    expect(screen.getByTestId('terminal-panel')).toHaveTextContent(firstKey);
    fireEvent.click(screen.getAllByRole('button', { name: 'Close Terminal' })[1]);
    expect(mocks.closeTerminalForTab).toHaveBeenCalledWith(secondKey);
  });

  it('refetches git status when the conversation revision changes', async () => {
    const { rerender } = render(<WorkspacePanel busy={false} revision="proj:conv-1" />);
    fireEvent.keyDown(window, { key: 'G', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(mocks.workspaceGit).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('Working tree clean')).toBeInTheDocument();
    // Same workspace path, different conversation: the panel must refresh.
    mocks.workspaceGit.mockResolvedValue({ branch: 'dev', branches: ['main', 'dev'], status: 'M src/app.ts', diff: '+changed' });
    rerender(<WorkspacePanel busy={false} revision="proj:conv-2" />);
    await waitFor(() => expect(mocks.workspaceGit).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('M src/app.ts')).toBeInTheDocument();
    expect(await screen.findByText('+changed')).toBeInTheDocument();
  });
});
