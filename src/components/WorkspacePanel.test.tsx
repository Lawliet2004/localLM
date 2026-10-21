import { act, render, screen, fireEvent, waitFor, within } from '@testing-library/react';
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
  editorRead: vi.fn(),
  editorSave: vi.fn(),
  closeHandlers: [] as ((event: { preventDefault: () => void }) => Promise<void>)[],
  destroy: vi.fn(),
  terminalMounts: [] as string[],
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    getWorkspace: mocks.getWorkspace,
    workspaceGit: mocks.workspaceGit,
    setWorkspace: mocks.setWorkspace,
    editorRead: mocks.editorRead,
    editorSave: mocks.editorSave,
  },
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: mocks.openPath }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onCloseRequested: async (handler: (event: { preventDefault: () => void }) => Promise<void>) => { mocks.closeHandlers.push(handler); return () => { mocks.closeHandlers = mocks.closeHandlers.filter(item => item !== handler); }; }, destroy: mocks.destroy }) }));
vi.mock('./CodeEditor', () => ({ CodeEditor: ({ initialContent, onChange }: { initialContent: string; onChange: (value: string) => void }) => <textarea aria-label="Code" defaultValue={initialContent} onChange={event => onChange(event.target.value)} /> }));
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
vi.mock('./BrowserPanel', () => ({ BrowserPanel: () => <div data-testid="browser-panel" />, closeBrowserForTab: vi.fn().mockResolvedValue(undefined) }));
vi.mock('./EditorTree', () => ({
  WorkspaceTree: ({ onOpenFile, hidden }: { onOpenFile: (path: string) => void; hidden: boolean }) => <div hidden={hidden} data-testid="workspace-tree"><button onClick={() => onOpenFile('main.ts')}>main.ts</button></div>,
}));

describe('WorkspacePanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.terminalMounts.length = 0;
    mocks.closeHandlers.length = 0;
    mocks.getWorkspace.mockResolvedValue({ path: 'C:/workspace' });
    mocks.workspaceGit.mockResolvedValue({ branch: 'main', branches: ['main', 'dev'], status: '', diff: '' });
    mocks.editorRead.mockResolvedValue({ content: 'original', sha256: 'initial' });
    mocks.editorSave.mockResolvedValue({ content: 'changed', sha256: 'saved' });
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
    for (const label of ['Review', 'Terminal', 'Browser', 'Editor']) {
      expect(within(launcher).getByRole('button', { name: new RegExp(`^${label}`) })).toBeInTheDocument();
    }
    await user.click(within(launcher).getByRole('button', { name: /^Editor/ }));
    expect(await screen.findByTestId('workspace-tree')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /^Editor/ })).toBeInTheDocument();
    // The + menu launches a second feature tab while the first stays open.
    await user.click(document.querySelector<HTMLElement>('.workspace-add summary')!);
    await user.click(within(document.querySelector<HTMLElement>('.workspace-add nav')!).getByRole('button', { name: /^Browser/ }));
    expect(await screen.findByTestId('browser-panel')).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /^Editor/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /^New tab/ })).toBeInTheDocument();
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
    expect(document.querySelector('.workspace-panel.panel-expanded')).toBeNull();
    expect(screen.getByRole('complementary', { name: 'Workspace inspector' })).toBeVisible();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('complementary', { name: 'Workspace inspector' })).not.toBeInTheDocument();
  });

  it('keeps the chat mounted beside the panel and restores the resized split after expansion', async () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const { unmount } = render(<><textarea aria-label="Chat draft" defaultValue="Keep this draft" /><WorkspacePanel host={host} busy={false} revision="p:c" /></>);
    try {
      fireEvent.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
      const panel = screen.getByRole('complementary', { name: 'Workspace inspector' });
      expect(host).toContainElement(panel);
      const divider = screen.getByRole('separator', { name: 'Resize workspace panel' });
      fireEvent.keyDown(divider, { key: 'End' });
      expect(divider).toHaveAttribute('aria-valuenow', '70');
      expect(panel.style.getPropertyValue('--workspace-panel-size')).toBe('70%');
      fireEvent.click(screen.getAllByRole('button', { name: 'Expand panel' })[0]);
      expect(panel).toHaveClass('panel-expanded');
      expect(screen.getByLabelText('Chat draft')).toHaveValue('Keep this draft');
      fireEvent.click(screen.getAllByRole('button', { name: 'Restore panel size' })[0]);
      expect(panel).not.toHaveClass('panel-expanded');
      expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '70');
      fireEvent.click(screen.getByRole('button', { name: 'Close inspector' }));
      fireEvent.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
      expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '70');
      fireEvent.doubleClick(screen.getByRole('separator'));
      expect(screen.getByRole('separator')).toHaveAttribute('aria-valuenow', '45');
    } finally { unmount(); host.remove(); }
  });

  it('resizes by dragging and stops when pointer capture is lost', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    const bounds = vi.spyOn(document.body, 'getBoundingClientRect').mockReturnValue({ width: 1000, height: 800, left: 0, top: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) });
    const { unmount } = render(<WorkspacePanel host={host} busy={false} revision="p:c" />);
    try {
      fireEvent.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
      const divider = screen.getByRole('separator');
      Object.assign(divider, { setPointerCapture: vi.fn(), releasePointerCapture: vi.fn() });
      const pointer = (type: string, clientX: number) => {
        const event = new Event(type, { bubbles: true });
        Object.assign(event, { pointerId: 1, button: 0, clientX });
        fireEvent(divider, event);
      };
      pointer('pointerdown', 550);
      pointer('pointermove', 400);
      expect(divider).toHaveAttribute('aria-valuenow', '60');
      pointer('pointermove', 0);
      expect(divider).toHaveAttribute('aria-valuenow', '70');
      pointer('lostpointercapture', 0);
      pointer('pointermove', 700);
      expect(divider).toHaveAttribute('aria-valuenow', '70');
    } finally { unmount(); host.remove(); bounds.mockRestore(); }
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
    fireEvent.click(screen.getAllByRole('tab')[0]);
    await waitFor(() => expect(mocks.terminalMounts).toHaveLength(3));
    expect(mocks.terminalMounts[2]).toBe(firstKey);
    expect(screen.getByTestId('terminal-panel')).toHaveTextContent(firstKey);
    fireEvent.click(screen.getAllByRole('button', { name: 'Close Terminal' })[1]);
    await waitFor(() => expect(mocks.closeTerminalForTab).toHaveBeenCalledWith(secondKey));
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

  it('keeps unsaved edits while Settings hides the inspector and protects tab close', async () => {
    const user = userEvent.setup();
    const { rerender } = render(<div><WorkspacePanel busy={false} revision="p:c" /></div>);
    fireEvent.keyDown(window, { key: 'p', ctrlKey: true });
    await user.click(screen.getByRole('button', { name: 'main.ts' }));
    fireEvent.change(await screen.findByLabelText('Code'), { target: { value: 'changed' } });
    rerender(<div style={{ display: 'none' }}><WorkspacePanel busy={false} revision="p:c" visible={false} /></div>);
    expect(screen.queryByRole('complementary', { name: 'Workspace inspector' })).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.closeHandlers).toHaveLength(1));
    act(() => { void mocks.closeHandlers[0]({ preventDefault: vi.fn() }); });
    expect(await screen.findByRole('dialog')).toBeVisible();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(mocks.destroy).not.toHaveBeenCalled();
    rerender(<div><WorkspacePanel busy={false} revision="p:c" visible /></div>);
    expect(screen.getByLabelText('Code')).toHaveValue('changed');
    expect(mocks.editorRead).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole('button', { name: 'Close main.ts' }));
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByLabelText('Code')).toHaveValue('changed');
    await user.click(screen.getByRole('button', { name: 'Close main.ts' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: /main.ts/ })).not.toBeInTheDocument());
    expect(mocks.editorSave).toHaveBeenCalledWith('C:/workspace', 'main.ts', 'changed', 'initial');
  });
});
