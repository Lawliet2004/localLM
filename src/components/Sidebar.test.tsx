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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
        workspace={{ projects: [], tasks: {} }}
      />
    );

    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));

    expect(mocks.open).toHaveBeenCalled();
    await waitFor(() => expect(mocks.saveProject).toHaveBeenCalledWith({ id: 'gamma', name: 'gamma', path: 'C:/projects/gamma' }));
    await waitFor(() => expect(onNew).toHaveBeenCalledWith('gamma'));
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
        onCollapse={vi.fn()}
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
        onCollapse={vi.fn()}
        workspace={sampleWorkspace}
        onProject={onProject}
      />
    );

    const folderHeader = screen.getByText('Alpha Project');
    await userEvent.click(folderHeader);

    // Previously, clicking a folder header called onProject(projectId) which caused bugs
    expect(onProject).not.toHaveBeenCalled();
  });

  it('renders localLM brand button and navigates through feature toolsets', async () => {
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
        onCollapse={vi.fn()}
        workspace={sampleWorkspace}
      />
    );

    // Workspace brand button
    const brandBtn = screen.getByRole('button', { name: 'Workspace menu' });
    expect(brandBtn).toHaveTextContent('localLM');

    // Open brand menu
    await userEvent.click(brandBtn);
    expect(await screen.findByRole('menuitem', { name: 'LocalLM Workspace' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Connectors' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Skills' })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: 'Tools & Permissions' })).toBeInTheDocument();

    // Verify toolsets navigation buttons
    const connectorsBtn = screen.getByRole('button', { name: 'Connectors' });
    const skillsBtn = screen.getByRole('button', { name: 'Skills' });
    const toolsBtn = screen.getByRole('button', { name: 'Tools' });
    const modelsBtn = screen.getByRole('button', { name: 'Models & runtime' });
    const executionBtn = screen.getByRole('button', { name: 'Execution' });

    expect(connectorsBtn).toBeInTheDocument();
    expect(skillsBtn).toBeInTheDocument();
    expect(toolsBtn).toBeInTheDocument();
    expect(modelsBtn).toBeInTheDocument();
    expect(executionBtn).toBeInTheDocument();

    await userEvent.click(connectorsBtn);
    expect(onPage).toHaveBeenCalledWith('connectors');

    await userEvent.click(skillsBtn);
    expect(onPage).toHaveBeenCalledWith('skills');

    await userEvent.click(toolsBtn);
    expect(onPage).toHaveBeenCalledWith('tools');

    await userEvent.click(modelsBtn);
    expect(onPage).toHaveBeenCalledWith('models');

    await userEvent.click(executionBtn);
    expect(onPage).toHaveBeenCalledWith('execution');
  });
});

