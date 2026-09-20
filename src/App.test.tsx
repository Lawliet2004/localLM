import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: async () => false, onResized: async () => () => {} }) }));
import { defaultRuntimeConfig } from './lib/types';
const mocks = vi.hoisted(() => ({
  save: vi.fn(), exportConversation: vi.fn(), bootstrap: vi.fn(),
  saveRememberedTools: vi.fn(), saveConversationTools: vi.fn(), conversationTools: vi.fn(),
  setWorkspace: vi.fn(), workspaceIndex: vi.fn(), removeProject: vi.fn(), confirm: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save, open: vi.fn(), confirm: mocks.confirm }));
vi.mock('./lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
  bootstrap: mocks.bootstrap, exportConversation: mocks.exportConversation,
  compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
  saveRememberedTools: mocks.saveRememberedTools, saveConversationTools: mocks.saveConversationTools, conversationTools: mocks.conversationTools,
  setWorkspace: mocks.setWorkspace, workspaceIndex: mocks.workspaceIndex, removeProject: mocks.removeProject,
  listConnectors: async () => [{ id: 'example', description: 'Example service', url: '', authType: 'apiKey', connected: true, hasCredential: true, tools: [{ name: 'kept_tool', description: 'Kept for new chats', inputSchema: {} }] }],
  listSkills: async () => [], getWorkspace: async () => ({ path: 'C:/workspace' }), hasDaytonaKey: async () => false,
  messages: async () => [{ id: 'message', conversationId: 'chat', role: 'user', content: 'Export this message', reasoning: '', status: 'complete', createdAt: 0 }],
} }));
const baseBootstrap = { conversations: [{ id: 'chat', title: 'Saved chat', updatedAt: 0 }], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 100, systemPrompt: '' }, runtime: { phase: 'stopped', message: '', modelPath: null },
  providers: [], preferredModel: { providerId: null, modelId: '' } };
beforeEach(() => {
  vi.clearAllMocks();
  mocks.confirm.mockResolvedValue(true);
  mocks.workspaceIndex.mockResolvedValue({ projects: [], tasks: {} });
  mocks.setWorkspace.mockResolvedValue(undefined);
  mocks.removeProject.mockResolvedValue({ projects: [], tasks: {} });
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [] } });
  mocks.conversationTools.mockResolvedValue({ sources: [], tools: [], accessMode: 'ask' });
  mocks.saveRememberedTools.mockResolvedValue(undefined);
  mocks.saveConversationTools.mockResolvedValue(undefined);
  mocks.exportConversation.mockResolvedValue(undefined);
});
async function openChat() {
  render(<App />);
  await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
  await screen.findByText('Export this message');
}
describe('desktop conversation export', () => {
  it('saves to the chosen native path and reports completion', async () => {
    mocks.save.mockResolvedValue('C:/exports/chat.json');
    await openChat();
    await userEvent.click(screen.getByRole('button', { name: 'Export conversation' }));
    expect(mocks.save).toHaveBeenCalledWith(expect.objectContaining({ filters: expect.arrayContaining([expect.objectContaining({ extensions: ['json'] })]) }));
    expect(mocks.exportConversation).toHaveBeenCalledWith('chat', 'C:/exports/chat.json');
    expect(await screen.findByRole('status')).toHaveTextContent('Saved conversation to C:/exports/chat.json');
  });
  it('does not write a file when the save dialog is cancelled', async () => {
    mocks.save.mockResolvedValue(null);
    await openChat();
    await userEvent.click(screen.getByRole('button', { name: 'Export conversation' }));
    expect(mocks.exportConversation).not.toHaveBeenCalled();
    expect(screen.queryByText(/Saved conversation to/)).not.toBeInTheDocument();
  });
  it('shows write errors without claiming success', async () => {
    mocks.save.mockResolvedValue('C:/exports/chat.md');
    mocks.exportConversation.mockRejectedValue(new Error('The disk is full.'));
    await openChat();
    await userEvent.click(screen.getByRole('button', { name: 'Export conversation' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('The disk is full.');
    expect(screen.queryByText(/Saved conversation to/)).not.toBeInTheDocument();
  });
});
describe('conversation permission modes', () => {
  it('shows full access clearly and carries it into a new conversation', async () => {
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    const mode = screen.getByRole('combobox', { name: 'Permission mode' });
    expect(mode).toHaveValue('ask');
    await userEvent.selectOptions(mode, 'fullAccess');
    expect(mode).toHaveValue('fullAccess');
    expect(screen.getByText('Selected tools run without prompts, including code and external changes.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    expect(mode).toHaveValue('fullAccess');
    await userEvent.selectOptions(mode, 'autoApprove');
    expect(screen.getByText('Workspace reads run automatically. Other actions ask.')).toBeInTheDocument();
  });
  it.each(['ask', 'autoApprove', 'fullAccess'])('restores saved %s permission mode after restarting', async accessMode => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [], accessMode } });
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    await waitFor(() => expect(screen.getByRole('combobox', { name: 'Permission mode' })).toHaveValue(accessMode));
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('persists permission-only changes and an explicit return to asking', async () => {
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    const mode = screen.getByRole('combobox', { name: 'Permission mode' });
    for (const accessMode of ['fullAccess', 'autoApprove', 'ask']) {
      await userEvent.selectOptions(mode, accessMode);
      await waitFor(() => expect(mocks.saveRememberedTools).toHaveBeenLastCalledWith({ sources: [], tools: [], accessMode }));
      await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
      expect(mode).toHaveValue(accessMode);
    }
  });
});

describe('remembered tool selection', () => {
  it('restores non-workspace remembered selections for an unscoped new chat at startup', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: ['__workspace'], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).not.toBeChecked());
    expect(screen.getByText('Tools · 1 selected')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('saves a change in a fresh chat immediately as the remembered selection', async () => {
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeEnabled());
    await userEvent.click(workspace);
    await waitFor(() => expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: ['__workspace'], tools: [], accessMode: 'ask' }));
    expect(screen.getByText('Tools · 5 selected')).toBeInTheDocument();
    // No conversation exists before the first message, so nothing is written per conversation.
    expect(mocks.saveConversationTools).not.toHaveBeenCalled();
  });
  it('reports a failed save without claiming the choice was applied', async () => {
    mocks.saveRememberedTools.mockRejectedValue(new Error('The disk is full.'));
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeEnabled());
    await userEvent.click(workspace);
    expect(await screen.findByRole('alert')).toHaveTextContent('Your tool selection was not saved: The disk is full.');
    expect(workspace).not.toBeChecked();
    expect(screen.getByText('Tools · Off')).toBeInTheDocument();
  });
  it('a new conversation inherits the remembered selection without writing it again', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 1 selected');
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByText('Tools · 1 selected')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('leaves workspace-only tools off for an unscoped new conversation', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: ['__workspace', '__execution'], tools: [] } });
    render(<App />);
    expect(await screen.findByRole('textbox', { name: 'Message' })).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('checkbox', { name: 'Workspace files' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Local code' })).not.toBeChecked();

    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('checkbox', { name: 'Workspace files' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Local code' })).not.toBeChecked();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('existing conversations keep their own selections and leave the remembered one alone', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 1 selected');
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeChecked());
    expect(screen.getByText('Tools · 5 selected')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
    // Opening another chat never changes the future-chat default.
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByText('Tools · 1 selected')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('changing tools in a saved conversation updates it and the remembered selection', async () => {
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 5 selected');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Workspace files' }));
    await waitFor(() => expect(mocks.saveConversationTools).toHaveBeenCalledWith('chat', { sources: [], tools: [], accessMode: 'ask' }));
    expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: [], tools: [], accessMode: 'ask' });
    expect(screen.getByText('Tools · Off')).toBeInTheDocument();
  });
});

describe('tools navigation', () => {
  it('keeps setup out of chat and preserves the draft when returning from Tools', async () => {
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    const input = screen.getByRole('textbox', { name: 'Message' });
    await userEvent.type(input, 'Keep my draft');
    expect(screen.queryByRole('checkbox', { name: 'Workspace files' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('heading', { name: 'Tools' })).toBeVisible();
    expect(screen.getByRole('checkbox', { name: 'Workspace files' })).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Back to chat' }));
    expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Keep my draft');
  });
});

describe('project folders and scoping', () => {
  it('starts a new chat scoped to a chosen project folder', async () => {
    mocks.workspaceIndex.mockResolvedValue({
      projects: [{ id: 'proj-1', name: 'Web Engine', path: 'C:/web-engine' }],
      tasks: {},
    });
    render(<App />);
    const newChatInFolder = await screen.findByRole('button', { name: 'New chat in Web Engine' });
    await userEvent.click(newChatInFolder);

    expect(mocks.setWorkspace).toHaveBeenCalledWith('C:/web-engine');
    const projectSelect = screen.getByRole('combobox', { name: 'Task project' });
    expect(projectSelect).toHaveValue('proj-1');
  });

  it('deletes a folder from the project section and resets active project', async () => {
    mocks.workspaceIndex.mockResolvedValue({
      projects: [{ id: 'proj-1', name: 'Web Engine', path: 'C:/web-engine' }],
      tasks: {},
    });
    mocks.removeProject.mockResolvedValue({
      projects: [],
      tasks: {},
    });
    render(<App />);
    const optionsBtn = await screen.findByRole('button', { name: 'Options for Web Engine' });
    await userEvent.click(optionsBtn);

    const deleteBtn = await screen.findByRole('menuitem', { name: 'Delete folder Web Engine' });
    await userEvent.click(deleteBtn);

    expect(mocks.confirm).toHaveBeenCalledWith(
      expect.stringContaining('Remove "Web Engine" from the project section?'),
      expect.objectContaining({ title: 'Remove project folder?', kind: 'warning' })
    );
    await waitFor(() => expect(mocks.removeProject).toHaveBeenCalledWith('proj-1'));
  });
});
