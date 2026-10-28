import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: async () => false, onResized: async () => () => {} }) }));
import type { ChatEvent, Message } from './lib/types';
import { toolOutput } from './lib/toolOutput';
import { defaultRuntimeConfig } from './lib/types';
const mocks = vi.hoisted(() => ({
  messages: vi.fn(), sendMessage: vi.fn(),
  save: vi.fn(), exportConversation: vi.fn(), bootstrap: vi.fn(),
  saveRememberedTools: vi.fn(), saveConversationTools: vi.fn(), conversationTools: vi.fn(),
  setWorkspace: vi.fn(), workspaceIndex: vi.fn(), removeProject: vi.fn(), confirm: vi.fn(),
  saveTaskMeta: vi.fn(), deleteConversation: vi.fn(),
  getPreset: vi.fn(), setPreset: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save, open: vi.fn(), confirm: mocks.confirm }));
vi.mock('./lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
  bootstrap: mocks.bootstrap, exportConversation: mocks.exportConversation,
  compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
  saveRememberedTools: mocks.saveRememberedTools, saveConversationTools: mocks.saveConversationTools, conversationTools: mocks.conversationTools,
  setWorkspace: mocks.setWorkspace, workspaceIndex: mocks.workspaceIndex, removeProject: mocks.removeProject,
  saveTaskMeta: mocks.saveTaskMeta, deleteConversation: mocks.deleteConversation,
  getPreset: mocks.getPreset, setPreset: mocks.setPreset,
  listConnectors: async () => [{ id: 'example', description: 'Example service', url: '', authType: 'apiKey', connected: true, hasCredential: true, tools: [{ name: 'kept_tool', description: 'Kept for new chats', inputSchema: {} }] }],
  listSkills: async () => [], getWorkspace: async () => ({ path: 'C:/workspace' }), hasDaytonaKey: async () => false,
  messages: mocks.messages, sendMessage: mocks.sendMessage,
  contextPreflight: async () => ({ available: false }),
} }));
const baseBootstrap = { conversations: [{ id: 'chat', title: 'Saved chat', updatedAt: 0 }], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 100, systemPrompt: '' }, runtime: { phase: 'stopped', message: '', modelPath: null },
  providers: [], preferredModel: { providerId: null, modelId: '' } };
beforeEach(() => {
  vi.clearAllMocks();
  mocks.messages.mockResolvedValue([{ id: 'message', conversationId: 'chat', role: 'user', content: 'Export this message', reasoning: '', status: 'complete', createdAt: 0 }]);
  localStorage.removeItem('locallm-appearance');
  mocks.confirm.mockResolvedValue(true);
  mocks.workspaceIndex.mockResolvedValue({ projects: [], tasks: {} });
  mocks.setWorkspace.mockResolvedValue(undefined);
  mocks.removeProject.mockResolvedValue({ projects: [], tasks: {} });
  mocks.saveTaskMeta.mockImplementation(async (id: string, meta: unknown) => {
    const index = await mocks.workspaceIndex();
    return { projects: index.projects, tasks: { ...index.tasks, [id]: meta } };
  });
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [] } });
  mocks.conversationTools.mockResolvedValue({ sources: [], tools: [], accessMode: 'ask' });
  mocks.saveRememberedTools.mockResolvedValue(undefined);
  mocks.saveConversationTools.mockResolvedValue(undefined);
  mocks.exportConversation.mockResolvedValue(undefined);
  mocks.getPreset.mockResolvedValue({ id: 'standard' });
  mocks.setPreset.mockResolvedValue(undefined);
});
async function openChat() {
  render(<App />);
  await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
  await screen.findByText('Export this message');
}
describe('brand home navigation', () => {
  it('returns from a saved conversation to home without requiring a project', async () => {
    await openChat();
    const home = screen.getByRole('button', { name: 'localLM home' });
    expect(home).not.toHaveAttribute('aria-expanded');
    expect(home.querySelector('.lucide-chevron-down')).toBeNull();
    await userEvent.click(home);
    expect(screen.queryByText('Export this message')).not.toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeVisible();
    expect(screen.queryByText('Add a project folder first, then start a chat.')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    expect(await screen.findByText('Export this message')).toBeInTheDocument();
  });
});
describe('workspace panel chrome', () => {
  it('shares the full canvas height with chat and shows one set of panel actions', async () => {
    await openChat();
    const message = screen.getByRole('textbox', { name: 'Message' });
    const conversation = message.closest('.workspace-conversation')!;
    const header = conversation.querySelector('.workspace-header')!;
    expect(header).not.toBeNull();
    await userEvent.type(message, 'Keep my draft');
    await userEvent.click(screen.getByRole('button', { name: 'Toggle workspace panel' }));
    const panel = screen.getByRole('complementary', { name: 'Workspace inspector' });
    expect(conversation.parentElement).toHaveClass('workspace-split');
    expect(panel.parentElement?.parentElement).toBe(conversation.parentElement);
    expect(screen.queryByRole('button', { name: 'Review changes' })).not.toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Expand panel' })).toHaveLength(1);
    expect(screen.queryByRole('button', { name: 'Toggle terminal' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open terminal in panel' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Toggle workspace panel' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Close inspector' }));
    expect(screen.getByRole('button', { name: 'Toggle workspace panel' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Review changes' })).not.toBeInTheDocument();
    expect(message).toHaveValue('Keep my draft');
  });
});
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
    expect(screen.getByText('Selected tools run automatically unless their settings require approval.')).toBeInTheDocument();
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
  it.each([['code', /^Code/], ['minimal', /^Minimal/], ['creator', /^Creator/]] as const)('saves the advanced %s profile for a conversation and restores it after remount', async (id, name) => {
    const first = render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
    await screen.findByText('Export this message');
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await userEvent.click(await screen.findByRole('radio', { name }));
    expect(mocks.setPreset).toHaveBeenCalledExactlyOnceWith('chat', id);
    first.unmount();
    mocks.getPreset.mockResolvedValue({ id });
    await openChat();
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('radio', { name })).toBeChecked();
  });
  it('restores non-workspace remembered selections for an unscoped new chat at startup', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: ['__workspace'], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).not.toBeChecked());
    expect(screen.getByText('Tools · 1 connector tool')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('saves a change in a fresh chat immediately as the remembered selection', async () => {
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeEnabled());
    await userEvent.click(workspace);
    await waitFor(() => expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: ['__workspace'], tools: [], accessMode: 'ask' }));
    expect(screen.getByText('Tools · 1 source')).toBeInTheDocument();
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
    expect(screen.getByText('Tools · No sources selected')).toBeInTheDocument();
  });
  it('a new conversation inherits the remembered selection without writing it again', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 1 connector tool');
    await userEvent.click(screen.getByRole('button', { name: 'Conversations' }));
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByText('Tools · 1 connector tool')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('leaves workspace-only tools off for an unscoped new conversation', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: ['__workspace', '__execution'], tools: [] } });
    render(<App />);
    expect(await screen.findByRole('textbox', { name: 'Message' })).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('checkbox', { name: 'Workspace files' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Local code' })).not.toBeChecked();

    await userEvent.click(screen.getByRole('button', { name: 'Conversations' }));
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByRole('checkbox', { name: 'Workspace files' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Local code' })).not.toBeChecked();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('existing conversations keep their own selections and leave the remembered one alone', async () => {
    mocks.workspaceIndex.mockResolvedValue({
      projects: [{ id: 'proj-1', name: 'Web Engine', path: 'C:/web-engine' }],
      tasks: {},
    });
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 1 connector tool');
    await userEvent.click(screen.getByRole('button', { name: 'Conversations' }));
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeChecked());
    expect(screen.getByText('Tools · 1 source')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
    // Opening another chat never changes the future-chat default; a scoped
    // new chat turns workspace files and execution on automatically.
    await userEvent.click(screen.getByRole('button', { name: 'Conversations' }));
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    expect(await screen.findByText('Tools · 2 sources · 1 connector tool')).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: 'Workspace files' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'Local code' })).toBeChecked();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('changing tools in a saved conversation updates it and the remembered selection', async () => {
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await userEvent.click(screen.getByRole('button', { name: 'Conversations' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
    await userEvent.click(screen.getByRole('button', { name: 'Tools' }));
    await screen.findByText('Tools · 1 source');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Workspace files' }));
    await waitFor(() => expect(mocks.saveConversationTools).toHaveBeenCalledWith('chat', { sources: [], tools: [], accessMode: 'ask' }));
    expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: [], tools: [], accessMode: 'ask' });
    expect(screen.getByText('Tools · No sources selected')).toBeInTheDocument();
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

describe('reference-style settings', () => {
  it('filters settings by purpose and returns to the saved draft', async () => {
    render(<App />);
    const message = await screen.findByRole('textbox', { name: 'Message' });
    await userEvent.clear(message);
    await userEvent.type(message, 'Review the project structure');
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(screen.getByRole('heading', { name: 'General' })).toBeVisible();
    const search = screen.getByRole('searchbox', { name: 'Search settings' });
    await userEvent.type(search, 'gpu');
    expect(screen.getByRole('button', { name: 'Settings: Models & runtime' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Settings: General' })).not.toBeInTheDocument();
    await userEvent.clear(search);
    await userEvent.type(search, 'unmatched-setting');
    expect(screen.getByRole('status')).toHaveTextContent('No settings found');
    await userEvent.click(screen.getByRole('button', { name: 'Clear settings search' }));
    expect(screen.getByRole('button', { name: 'Settings: General' })).toHaveAttribute('aria-current', 'page');
    await userEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Review the project structure');
  });

  it('saves permission changes through General and shares the state with chat', async () => {
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await userEvent.click(screen.getByRole('radio', { name: 'Full access' }));
    await waitFor(() => expect(mocks.saveRememberedTools).toHaveBeenLastCalledWith({ sources: [], tools: [], accessMode: 'fullAccess' }));
    expect(screen.getByText(/Enabled tools run automatically unless their settings require approval/)).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    expect(screen.getByRole('combobox', { name: 'Permission mode' })).toHaveValue('fullAccess');
  });

  it('keeps the previous permission mode when a General settings save fails', async () => {
    mocks.saveRememberedTools.mockRejectedValue(new Error('Could not write settings.'));
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await userEvent.click(screen.getByRole('radio', { name: 'Full access' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Your tool selection was not saved: Could not write settings.');
    expect(screen.getByRole('radio', { name: 'Ask for approval' })).toBeChecked();
  });

  it('persists appearance changes and restores the chosen theme on mount', async () => {
    localStorage.setItem('locallm-theme', 'light');
    const app = render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await userEvent.click(screen.getByRole('button', { name: 'Settings: Appearance' }));
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Theme' }), 'dark');
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    expect(localStorage.getItem('locallm-theme')).toBe('dark');
    app.unmount();
    render(<App />);
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await userEvent.click(screen.getByRole('button', { name: 'Settings: Appearance' }));
    expect(screen.getByRole('combobox', { name: 'Theme' })).toHaveValue('dark');
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Theme' }), 'light');
  });

  it('finds Appearance by its controls and applies preferences to the saved conversation', async () => {
    await openChat();
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(screen.queryByRole('combobox', { name: 'Theme' })).not.toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search settings' }), 'font');
    await userEvent.click(screen.getByRole('button', { name: 'Settings: Appearance' }));
    await userEvent.click(screen.getByRole('button', { name: 'Ocean accent' }));
    await userEvent.selectOptions(screen.getByLabelText('Conversation font'), 'serif');
    await userEvent.selectOptions(screen.getByLabelText('Code size'), '16');
    await userEvent.selectOptions(screen.getByLabelText('Interface density'), 'compact');
    await userEvent.selectOptions(screen.getByLabelText('Conversation width'), 'wide');
    await userEvent.selectOptions(screen.getByLabelText('Animations'), 'reduced');
    expect(document.documentElement).toHaveAttribute('data-accent', 'blue');
    expect(document.documentElement).toHaveAttribute('data-density', 'compact');
    expect(document.documentElement).toHaveAttribute('data-chat-width', 'wide');
    expect(document.documentElement).toHaveAttribute('data-motion', 'reduced');
    expect(document.documentElement.style.getPropertyValue('--code-font-size')).toBe('16px');
    await userEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    expect(await screen.findByText('Export this message')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await userEvent.click(screen.getByRole('button', { name: 'Settings: Appearance' }));
    await userEvent.click(screen.getByRole('button', { name: 'Reset appearance' }));
    expect(screen.getByRole('button', { name: 'Reset appearance' })).toBeDisabled();
    expect(document.documentElement).toHaveAttribute('data-accent', 'forest');
  });
});

describe('settings as a floating modal', () => {
  it('opens as a dialog over the chat and closes via the close button', async () => {
    render(<App />);
    await screen.findByRole('textbox', { name: 'Message' });
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(await screen.findByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    // The chat surface stays mounted underneath the modal.
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    expect(screen.queryByRole('dialog', { name: 'Settings' })).not.toBeInTheDocument();
  });
  it('closes on Escape and returns focus to the conversation', async () => {
    render(<App />);
    await screen.findByRole('textbox', { name: 'Message' });
    await userEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    const dialog = await screen.findByRole('dialog', { name: 'Settings' });
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(dialog).not.toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Message' })).toBeInTheDocument();
  });
});

it('restores focus to the opener when the settings modal closes', async () => {
  render(<App />);
  await screen.findByRole('textbox', { name: 'Message' });
  const trigger = screen.getByRole('button', { name: 'Open settings' });
  trigger.focus();
  await userEvent.click(trigger);
  const dialog = await screen.findByRole('dialog', { name: 'Settings' });
  expect(dialog.contains(document.activeElement)).toBe(true);
  fireEvent.keyDown(dialog, { key: 'Escape' });
  expect(dialog).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();
});


it('streams tool-local channels and reconciles completion to persisted history', async () => {
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, runtime: { phase: 'ready', message: '', modelPath: 'fixture.gguf' }, rememberedTools: { sources: [], tools: [] } });
  let receive!: (event: ChatEvent) => void;
  let finish!: () => void;
  mocks.sendMessage.mockImplementation((_id: string, _content: string, callback: (event: ChatEvent) => void) => {
    receive = callback;
    return new Promise<void>(resolve => { finish = resolve; });
  });
  await openChat();
  fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), { target: { value: 'Run checks' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
  await waitFor(() => expect(mocks.sendMessage).toHaveBeenCalledOnce());
  const emit = (toolStream: NonNullable<ChatEvent['toolStream']>) => receive({ messageId: '', content: '', reasoning: '', toolStream });
  await act(async () => {
    emit({ type: 'started', toolCallId: 'fixture', toolName: 'execute_command', command: 'npm test' });
    emit({ type: 'outputChunk', toolCallId: 'fixture', stream: 'stdout', chunk: 'test passed' });
    emit({ type: 'outputChunk', toolCallId: 'fixture', stream: 'stderr', chunk: 'warning only' });
  });
  await waitFor(() => expect(screen.getByTestId('terminal-viewport')).toHaveTextContent('test passed'));
  expect(screen.getByTestId('terminal-viewport')).toHaveTextContent('warning only');
  await act(async () => { emit({ type: 'finished', toolCallId: 'fixture', exitCode: 0, durationMs: 123 }); });
  expect(screen.getByTestId('status-exit-0')).toBeInTheDocument();
  expect(screen.getByTestId('terminal-viewport')).toHaveTextContent('test passed');
  const saved: Message = { id: 'saved-tool', conversationId: 'chat', role: 'tool', content: JSON.stringify({ request: { name: 'execute_command', arguments: { command: 'npm test' } }, result: { stdout: 'test passed', stderr: 'warning only', exitCode: 0, durationMs: 123 } }), status: 'complete', reasoning: '', createdAt: 1 };
  mocks.messages.mockResolvedValue([saved]);
  await act(async () => { finish(); });
  expect(toolOutput.read('chat', 'tool-fixture')).toEqual({ stdout: '', stderr: '' });
  expect(screen.getAllByTestId('command-run-card')).toHaveLength(1);
  expect(screen.getByTestId('status-exit-0')).toBeInTheDocument();
});

it('keeps live output separate when provider call IDs repeat in different steps', async () => {
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, runtime: { phase: 'ready', message: '', modelPath: 'fixture.gguf' }, rememberedTools: { sources: [], tools: [] } });
  let receive!: (event: ChatEvent) => void;
  let finish!: () => void;
  mocks.sendMessage.mockImplementation((_id: string, _content: string, callback: (event: ChatEvent) => void) => {
    receive = callback;
    return new Promise<void>(resolve => { finish = resolve; });
  });
  await openChat();
  fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), { target: { value: 'Run twice' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
  await waitFor(() => expect(mocks.sendMessage).toHaveBeenCalledOnce());
  const emit = (stepId: string, toolStream: NonNullable<ChatEvent['toolStream']>) => receive({ messageId: '', content: '', reasoning: '', runId: 'run', stepId, toolStream });
  await act(async () => {
    emit('one', { type: 'started', toolCallId: 'reused', toolName: 'execute_command', command: 'echo first' });
    emit('one', { type: 'outputChunk', toolCallId: 'reused', stream: 'stdout', chunk: 'first output' });
    emit('one', { type: 'finished', toolCallId: 'reused', exitCode: 0, durationMs: 10 });
    emit('two', { type: 'started', toolCallId: 'reused', toolName: 'execute_command', command: 'echo second' });
    emit('two', { type: 'outputChunk', toolCallId: 'reused', stream: 'stdout', chunk: 'second output' });
  });
  await waitFor(() => expect(screen.getAllByTestId('command-run-card')).toHaveLength(2));
  const cards = screen.getAllByTestId('command-run-card');
  fireEvent.click(screen.getByText('echo first'));
  expect(cards[0]).toHaveTextContent('first output');
  expect(cards[0]).not.toHaveTextContent('second output');
  expect(cards[1]).toHaveTextContent('second output');
  expect(cards[1]).not.toHaveTextContent('first output');
  await act(async () => {
    emit('two', { type: 'finished', toolCallId: 'reused', exitCode: 1, durationMs: 20 });
    finish();
  });
  const { toolStreamId } = await import('./lib/agentTranscript');
  for (const stepId of ['one', 'two']) expect(toolOutput.read('chat', toolStreamId({ conversationId: 'chat', runId: 'run', stepId, toolCallId: 'reused' }))).toEqual({ stdout: '', stderr: '' });
});
