import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Sidebar } from './Sidebar';
import type { WorkspaceIndex, Conversation } from '../lib/types';

const mocks = vi.hoisted(() => ({
  removeProject: vi.fn(),
  saveProject: vi.fn(),
  confirm: vi.fn(),
  open: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: mocks.confirm,
  open: mocks.open,
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    removeProject: mocks.removeProject,
    saveProject: mocks.saveProject,
    searchSessions: async () => [],
  },
}));

describe('Sidebar project folders and actions', () => {
  const sampleWorkspace: WorkspaceIndex = {
    projects: [
      { id: 'alpha-proj', name: 'Alpha Project', path: 'C:/projects/alpha' },
      { id: 'beta-proj', name: 'Beta Project', path: 'C:/projects/beta' },
    ],
    tasks: {
      'chat-1': { projectId: 'alpha-proj', archived: false, pinned: false },
      'chat-2': { projectId: null, archived: false, pinned: false },
    },
  };

  const sampleConversations: Conversation[] = [
    { id: 'chat-1', title: 'Alpha task chat', updatedAt: 1000, providerId: null, modelId: null, providerSelectionRequired: false },
    { id: 'chat-2', title: 'General unfiled chat', updatedAt: 2000, providerId: null, modelId: null, providerSelectionRequired: false },
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.confirm.mockResolvedValue(true);
    mocks.removeProject.mockImplementation(async (id: string) => ({
      projects: sampleWorkspace.projects.filter(p => p.id !== id),
      tasks: {
        ...sampleWorkspace.tasks,
        ...(id === 'alpha-proj' ? { 'chat-1': { projectId: null, archived: false, pinned: false } } : {}),
      },
    }));
  });

  it('renders only registered projects with no built-in folder', () => {
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId="chat-1"
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    expect(screen.getByText('Alpha Project')).toBeInTheDocument();
    expect(screen.getByText('Beta Project')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'New chat in localLM' })).not.toBeInTheDocument();
    expect(screen.queryByText('Documents')).not.toBeInTheDocument();
  });

  it('starts a new chat directly scoped to a particular project folder via + button', async () => {
    const onNew = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    const newChatAlphaBtn = screen.getByRole('button', { name: 'New chat in Alpha Project' });
    await userEvent.click(newChatAlphaBtn);

    expect(onNew).toHaveBeenCalledTimes(1);
    expect(onNew).toHaveBeenCalledWith('alpha-proj');
  });

  it('starts a new chat via project options dropdown menu', async () => {
    const onNew = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    const optionsBtn = screen.getByRole('button', { name: 'Options for Beta Project' });
    await userEvent.click(optionsBtn);

    const newChatMenuItem = await screen.findByRole('menuitem', { name: 'New chat in Beta Project' });
    await userEvent.click(newChatMenuItem);

    expect(onNew).toHaveBeenCalledWith('beta-proj');
  });

  it('deletes a folder from the project section after confirmation', async () => {
    const onWorkspace = vi.fn();
    const onProject = vi.fn();
    const onDeleteConversations = vi.fn();

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId="chat-1"
        projectId="alpha-proj"
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
        onWorkspace={onWorkspace}
        onProject={onProject}
        onDeleteConversations={onDeleteConversations}
      />
    );

    const optionsBtn = screen.getByRole('button', { name: 'Options for Alpha Project' });
    await userEvent.click(optionsBtn);

    const deleteBtn = await screen.findByRole('menuitem', { name: 'Delete folder Alpha Project' });
    await userEvent.click(deleteBtn);

    expect(mocks.confirm).toHaveBeenCalledWith(
      expect.stringContaining('Remove "Alpha Project" from the project section?'),
      expect.objectContaining({ title: 'Remove project folder?', kind: 'warning' })
    );

    await waitFor(() => expect(mocks.removeProject).toHaveBeenCalledWith('alpha-proj'));
    expect(onWorkspace).toHaveBeenCalledWith(expect.objectContaining({
      projects: [{ id: 'beta-proj', name: 'Beta Project', path: 'C:/projects/beta' }],
    }));
    expect(onDeleteConversations).toHaveBeenCalledWith(['chat-1']);
    expect(onProject).toHaveBeenCalledWith(null);
  });

  it('cancelling the delete confirmation does not remove the project folder', async () => {
    mocks.confirm.mockResolvedValue(false);
    const onWorkspace = vi.fn();

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
        onWorkspace={onWorkspace}
      />
    );

    const optionsBtn = screen.getByRole('button', { name: 'Options for Alpha Project' });
    await userEvent.click(optionsBtn);

    const deleteBtn = await screen.findByRole('menuitem', { name: 'Delete folder Alpha Project' });
    await userEvent.click(deleteBtn);

    expect(mocks.confirm).toHaveBeenCalled();
    expect(mocks.removeProject).not.toHaveBeenCalled();
    expect(onWorkspace).not.toHaveBeenCalled();
  });

  it('offers delete and options controls on every folder', () => {
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        projectId="alpha-proj"
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    for (const name of ['Alpha Project', 'Beta Project']) {
      expect(screen.getByRole('button', { name: `Delete folder ${name}` })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: `Options for ${name}` })).toBeInTheDocument();
    }
    expect(screen.queryByRole('button', { name: 'Delete folder localLM' })).not.toBeInTheDocument();
  });

  it('renders unassigned chats flat without a folder', () => {
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    // chat-2 has projectId: null — it appears flat, outside any folder
    expect(screen.getByText('General unfiled chat')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'New chat in localLM' })).not.toBeInTheDocument();
  });

  it('scopes the top New chat button to the active folder', async () => {
    const onNew = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        projectId="beta-proj"
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));
    expect(onNew).toHaveBeenCalledWith('beta-proj');
  });

  it('scopes the top New chat button to the first folder when none is active', async () => {
    const onNew = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));
    expect(onNew).toHaveBeenCalledWith('alpha-proj');
  });

  it('opens the folder picker then starts a chat inside it when no folders exist', async () => {
    const onNew = vi.fn();
    mocks.open.mockResolvedValue('C:/projects/gamma');
    mocks.saveProject.mockImplementation(async (project: { id: string; name: string; path: string }) => ({
      projects: [project],
      tasks: {},
    }));
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={[]}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={{ projects: [], tasks: {} }}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));

    expect(mocks.open).toHaveBeenCalled();
    await waitFor(() => expect(mocks.saveProject).toHaveBeenCalled());
    const savedProject = mocks.saveProject.mock.calls[0][0] as { id: string; name: string; path: string };
    expect(savedProject).toMatchObject({ name: 'gamma', path: 'C:/projects/gamma' });
    // New projects get a unique generated id, never a basename-derived one (A02).
    expect(savedProject.id).not.toBe('gamma');
    expect(savedProject.id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i);
    await waitFor(() => expect(onNew).toHaveBeenCalledWith(savedProject.id));
  });

  it('assigns distinct ids to two folders that share the same basename (A02)', async () => {
    const onWorkspace = vi.fn();
    mocks.open
      .mockResolvedValueOnce('C:/one/api')
      .mockResolvedValueOnce('C:/two/api');
    mocks.saveProject.mockImplementation(async (project: { id: string; name: string; path: string }) => ({
      projects: [project],
      tasks: {},
    }));

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={[]}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={{ projects: [], tasks: {} }}
        onWorkspace={onWorkspace}
      />
    );

    const addBtn = screen.getByRole('button', { name: 'Add project' });
    await userEvent.click(addBtn);
    await waitFor(() => expect(mocks.saveProject).toHaveBeenCalledTimes(1));
    await userEvent.click(addBtn);
    await waitFor(() => expect(mocks.saveProject).toHaveBeenCalledTimes(2));

    const first = mocks.saveProject.mock.calls[0][0] as { id: string; name: string; path: string };
    const second = mocks.saveProject.mock.calls[1][0] as { id: string; name: string; path: string };
    expect(first).toMatchObject({ name: 'api', path: 'C:/one/api' });
    expect(second).toMatchObject({ name: 'api', path: 'C:/two/api' });
    expect(first.id).not.toBe(second.id);
    expect(first.id).not.toBe('api');
    expect(second.id).not.toBe('api');
  });

  it('does not start a chat or surface an id when the project save fails (A02)', async () => {
    const onNew = vi.fn();
    mocks.open.mockResolvedValue('C:/projects/gamma');
    mocks.saveProject.mockRejectedValue(new Error('project id already exists with a different path'));

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={[]}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={{ projects: [], tasks: {} }}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));
    await waitFor(() => expect(mocks.saveProject).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('project id already exists'));
    expect(onNew).not.toHaveBeenCalled();
  });

  it('re-adding an already registered folder path keeps the existing project untouched (A02)', async () => {
    const onWorkspace = vi.fn();
    mocks.open.mockResolvedValue('C:/projects/alpha');

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
        onWorkspace={onWorkspace}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    await waitFor(() => expect(mocks.open).toHaveBeenCalled());
    expect(mocks.saveProject).not.toHaveBeenCalled();
    expect(onWorkspace).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('stays usable when sidebar width storage throws (A05)', () => {
    const getSpy = vi.spyOn(Storage.prototype, 'getItem')
      .mockImplementation(() => { throw new DOMException('Access denied', 'SecurityError'); });
    const setSpy = vi.spyOn(Storage.prototype, 'setItem')
      .mockImplementation(() => { throw new DOMException('Quota exceeded', 'QuotaExceededError'); });
    try {
      render(
        <Sidebar
          page="chat"
          onPage={vi.fn()}
          conversations={sampleConversations}
          activeId={null}
          onSelect={vi.fn()}
          onNew={vi.fn()}
          busy={false}
          search=""
          onSearch={vi.fn()}
          theme="dark"
          onTheme={vi.fn()}
          workspace={sampleWorkspace}
        />
      );
      expect(document.querySelector('aside.sidebar')).not.toBeNull();
      expect(screen.getByRole('button', { name: 'New conversation' })).toBeEnabled();
      expect(screen.getByRole('button', { name: 'Alpha Project' })).toBeInTheDocument();
    } finally {
      getSpy.mockRestore();
      setSpy.mockRestore();
    }
  });

  it('exposes the project disclosure as a button with expanded state (A14)', async () => {
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    const disclosure = screen.getByRole('button', { name: 'Alpha Project' });
    // A real button — not a div — and it is not nested inside another button.
    expect(disclosure.tagName).toBe('BUTTON');
    expect(disclosure.closest('button')).toBe(disclosure);
    expect(disclosure.querySelector('button')).toBeNull();
    expect(disclosure).toHaveAttribute('aria-expanded', 'true');

    await userEvent.click(disclosure);
    expect(disclosure).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('button', { name: 'Alpha task chat' })).not.toBeInTheDocument();

    await userEvent.click(disclosure);
    expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('button', { name: 'Alpha task chat' })).toBeInTheDocument();
  });

  it('reaches project actions from the keyboard without hover (A14)', async () => {
    const onNew = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={onNew}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    const disclosure = screen.getByRole('button', { name: 'Beta Project' });
    disclosure.focus();

    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'New chat in Beta Project' }));
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Delete folder Beta Project' }));
    await userEvent.tab();
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Options for Beta Project' }));

    // The keyboard can start a chat in the second project without any hover.
    await userEvent.keyboard('{Enter}');
    expect(screen.getByRole('menuitem', { name: 'New chat in Beta Project' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('menuitem', { name: 'New chat in Beta Project' }));
    expect(onNew).toHaveBeenCalledWith('beta-proj');
  });

  it('allows deleting custom project folders directly via the trash icon button', async () => {
    const onWorkspace = vi.fn();
    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
        onWorkspace={onWorkspace}
      />
    );

    const directTrashBtn = screen.getByRole('button', { name: 'Delete folder Alpha Project' });
    await userEvent.click(directTrashBtn);

    expect(mocks.confirm).toHaveBeenCalledWith(
      expect.stringContaining('Remove "Alpha Project" from the project section?'),
      expect.objectContaining({ title: 'Remove project folder?', kind: 'warning' })
    );
    await waitFor(() => expect(mocks.removeProject).toHaveBeenCalledWith('alpha-proj'));
  });

  it('toggling folder collapse does not call onProject or change the active project', async () => {
    const onProject = vi.fn();

    render(
      <Sidebar
        page="chat"
        onPage={vi.fn()}
        conversations={sampleConversations}
        activeId="chat-1"
        projectId="alpha-proj"
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
        onProject={onProject}
      />
    );

    const folderHeader = screen.getByText('Alpha Project');
    await userEvent.click(folderHeader);

    // Previously, clicking a folder header called onProject(projectId) which caused bugs
    expect(onProject).not.toHaveBeenCalled();
  });

  it('renders the localLM brand button and navigates home', async () => {
    const onPage = vi.fn();

    render(
      <Sidebar
        page="chat"
        onPage={onPage}
        conversations={sampleConversations}
        activeId={null}
        onSelect={vi.fn()}
        onNew={vi.fn()}
        busy={false}
        search=""
        onSearch={vi.fn()}
        theme="dark"
        onTheme={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    const brandBtn = screen.getByRole('button', { name: 'localLM home' });
    expect(brandBtn).toHaveTextContent('localLM');

    await userEvent.click(brandBtn);
    expect(onPage).toHaveBeenCalledWith('chat');
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });
});

