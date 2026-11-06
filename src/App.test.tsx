import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: async () => false, onResized: async () => () => {} }) }));
import type { ActiveRunSnapshot, ChatEvent, Message, SessionEvent } from './lib/types';
import { toolOutput } from './lib/toolOutput';
import { saveDraft } from './lib/drafts';
import { defaultRuntimeConfig } from './lib/types';
const mocks = vi.hoisted(() => ({
  messages: vi.fn(), sendMessage: vi.fn(),
  save: vi.fn(), exportConversation: vi.fn(), bootstrap: vi.fn(),
  saveRememberedTools: vi.fn(), saveConversationTools: vi.fn(), conversationTools: vi.fn(),
  setWorkspace: vi.fn(), workspaceIndex: vi.fn(), removeProject: vi.fn(), confirm: vi.fn(),
  saveTaskMeta: vi.fn(), deleteConversation: vi.fn(), createConversation: vi.fn(),
  getPreset: vi.fn(), setPreset: vi.fn(),
  cancelGeneration: vi.fn(), resolveToolApproval: vi.fn(),
  getPendingTurn: vi.fn(), discardPendingTurn: vi.fn(),
  listActiveRuns: vi.fn(),
  getSessionEvents: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save, open: vi.fn(), confirm: mocks.confirm }));
vi.mock('./lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
  bootstrap: mocks.bootstrap, exportConversation: mocks.exportConversation,
  compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
  saveRememberedTools: mocks.saveRememberedTools, saveConversationTools: mocks.saveConversationTools, conversationTools: mocks.conversationTools,
  setWorkspace: mocks.setWorkspace, workspaceIndex: mocks.workspaceIndex, removeProject: mocks.removeProject,
  saveTaskMeta: mocks.saveTaskMeta, deleteConversation: mocks.deleteConversation, createConversation: mocks.createConversation,
  getPreset: mocks.getPreset, setPreset: mocks.setPreset,
  cancelGeneration: mocks.cancelGeneration, resolveToolApproval: mocks.resolveToolApproval,
  getPendingTurn: mocks.getPendingTurn, discardPendingTurn: mocks.discardPendingTurn,
  listActiveRuns: mocks.listActiveRuns,
  getSessionEvents: mocks.getSessionEvents,
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
  localStorage.removeItem('locallm-open-sessions');
  mocks.cancelGeneration.mockResolvedValue(undefined);
  mocks.resolveToolApproval.mockResolvedValue(undefined);
  mocks.getPendingTurn.mockResolvedValue(null);
  mocks.listActiveRuns.mockResolvedValue([]);
  mocks.getSessionEvents.mockResolvedValue([]);
  mocks.discardPendingTurn.mockResolvedValue(undefined);
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

describe('recovering active responses', () => {
  it('waits for the initial active-run check before allowing a new send', async () => {
    let finishCheck!: (runs: ActiveRunSnapshot[]) => void;
    mocks.listActiveRuns.mockImplementationOnce(() => new Promise(resolve => { finishCheck = resolve; }));
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, runtime: { phase: 'ready', message: '', modelPath: 'fixture.gguf' } });
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
    expect(await screen.findByText('Opening conversation…')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send message' })).toBeDisabled();
    await act(async () => finishCheck([{ conversationId: 'chat', instanceId: 'existing', phase: 'running', cancellationRequested: false,
      state: 'generating', activity: 'Working', approvals: [] }]));
    expect(await screen.findByRole('button', { name: 'Stop response' })).toBeInTheDocument();
    expect(mocks.sendMessage).not.toHaveBeenCalled();
  });

  it('keeps independently recovered conversations and their Stop actions separate', async () => {
    let snapshots: ActiveRunSnapshot[] = [
      { conversationId: 'chat', instanceId: 'run-a', phase: 'queued', cancellationRequested: false, state: 'queued', activity: 'Waiting for a slot', approvals: [] },
      { conversationId: 'other', instanceId: 'run-b', phase: 'running', cancellationRequested: false, state: 'generating', activity: 'Working', approvals: [] },
    ];
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, conversations: [...baseBootstrap.conversations, { id: 'other', title: 'Other chat', updatedAt: 0 }] });
    mocks.listActiveRuns.mockImplementation(async () => snapshots);
    mocks.messages.mockImplementation(async (id: string) => [{ id: `${id}-user`, conversationId: id, role: 'user', content: `${id} message`, reasoning: '', status: 'complete', createdAt: 0 }]);
    render(<App />);
    expect(await screen.findByText('Waiting for a slot')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    await screen.findByText('chat message');
    fireEvent.click(screen.getByRole('button', { name: 'Stop response' }));
    expect(mocks.cancelGeneration).toHaveBeenCalledWith('chat');
    expect(mocks.cancelGeneration).not.toHaveBeenCalledWith('other');
    snapshots = snapshots.filter(run => run.conversationId === 'other');
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Stop response' })).not.toBeInTheDocument(), { timeout: 2500 });
    await userEvent.click(screen.getByRole('button', { name: 'Other chat' }));
    await screen.findByText('other message');
    expect(screen.getByRole('button', { name: 'Stop response' })).toBeInTheDocument();
  });

  it('restores a pending approval, supports Stop, and shows the saved final response', async () => {
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.open = true; } });
    try {
    let snapshots: ActiveRunSnapshot[] = [{ conversationId: 'chat', instanceId: 'run-1', phase: 'running', cancellationRequested: false,
      state: 'awaiting_approval', activity: 'Waiting for approval', approvals: [{ id: 'approval-1', connector: 'Workspace', name: 'write', arguments: { path: 'app.tsx' } }] }];
    let saved: Message[] = [{ id: 'message', conversationId: 'chat', role: 'user', content: 'Export this message', reasoning: '', status: 'complete', createdAt: 0 },
      { id: 'response', conversationId: 'chat', role: 'assistant', content: 'Partial response', reasoning: '', status: 'streaming', createdAt: 1 }];
    mocks.listActiveRuns.mockImplementation(async () => snapshots);
    mocks.messages.mockImplementation(async () => saved);
    render(<App />);
    const dialog = await screen.findByRole('dialog', { name: 'Allow Workspace to run this tool?' });
    expect(dialog).toHaveTextContent('Workspace');
    expect(await screen.findByText('Input needed')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Allow once' }));
    expect(mocks.resolveToolApproval).toHaveBeenCalledWith('approval-1', true);
    snapshots = [{ ...snapshots[0], state: 'generating', approvals: [] }];
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Allow Workspace to run this tool?' })).not.toBeInTheDocument(), { timeout: 2500 });
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    expect(await screen.findByText('Partial response')).toBeInTheDocument();
    expect(screen.getByText(/Reconnected to a running response/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Stop response' }));
    expect(mocks.cancelGeneration).toHaveBeenCalledWith('chat');
    saved = [saved[0], { ...saved[1], content: 'Final saved response', status: 'complete' }];
    snapshots = [];
    expect(await screen.findByText('Final saved response', {}, { timeout: 2500 })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Stop response' })).not.toBeInTheDocument();
    expect(mocks.sendMessage).not.toHaveBeenCalled();
    } finally {
      if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original);
      else delete (HTMLDialogElement.prototype as Partial<HTMLDialogElement>).showModal;
    }
  });
});
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
  it.each([['code', /^Code \(PTC\)/], ['minimal', /^Minimal/], ['creator', /^Creator/]] as const)('saves the advanced %s profile for a conversation and restores it after remount', async (id, name) => {
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

  it('keeps a failed chat deletion visible after earlier folder deletions succeed', async () => {
    const conversations = [
      { id: 'first', title: 'First chat', updatedAt: 0 },
      { id: 'second', title: 'Second chat', updatedAt: 0 },
    ];
    localStorage.setItem('locallm-open-sessions', JSON.stringify(['first', 'second']));
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, conversations, rememberedTools: { sources: [], tools: [] } });
    mocks.workspaceIndex.mockResolvedValue({
      projects: [{ id: 'proj-1', name: 'Web Engine', path: 'C:/web-engine' }],
      tasks: Object.fromEntries(conversations.map(chat => [chat.id, { projectId: 'proj-1', archived: false, pinned: false }])),
    });
    mocks.deleteConversation.mockReset().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error('Second chat could not be deleted'));
    render(<App />);
    expect(await screen.findByRole('button', { name: 'First chat' })).toBeVisible();
    expect(screen.queryByRole('navigation', { name: 'Open conversations' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Options for Web Engine' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete folder Web Engine' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Second chat could not be deleted');
    expect(screen.queryByRole('button', { name: 'First chat' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Second chat' })).toBeVisible();
    expect(mocks.deleteConversation.mock.calls.map(([id]) => id)).toEqual(['first', 'second']);
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

// Native stream ownership is the critical regression for a multi-session UI.
describe('independent running conversations', () => {
  function fixture() {
    saveDraft('one', ''); saveDraft('two', '');
    const conversations = [{ id: 'one', title: 'Research notes', updatedAt: 1 }, { id: 'two', title: 'Draft document', updatedAt: 2 }];
    const history: Record<string, Message[]> = Object.fromEntries(conversations.map(c => [c.id, [{ id: `${c.id}-user`, conversationId: c.id, role: 'user', content: `${c.title} context`, reasoning: '', status: 'complete', createdAt: 1 }]]));
    const channels = new Map<string, (event: ChatEvent) => void>();
    const finish = new Map<string, () => void>();
    const reject = new Map<string, (error: Error) => void>();
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, conversations, runtime: { phase: 'ready', message: '', modelPath: 'fixture.gguf' }, rememberedTools: { sources: [], tools: [] } });
    mocks.messages.mockImplementation(async (id: string) => history[id] ?? []);
    mocks.sendMessage.mockImplementation((id: string, _text: string, callback: (event: ChatEvent) => void) => {
      channels.set(id, callback);
      return new Promise<void>((resolve, fail) => { finish.set(id, resolve); reject.set(id, fail); });
    });
    const emit = (id: string, content: string) => channels.get(id)!({ messageId: `${id}-answer`, content, reasoning: '' });
    async function submit(text: string) {
      fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), { target: { value: text } });
      fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
      await waitFor(() => expect(channels.size).toBeGreaterThan(0));
    }
    return { history, channels, finish, reject, emit, submit };
  }

  it('allows switching away while a new conversation is being created', async () => {
    const f = fixture();
    const index = { projects: [{ id: 'project', name: 'Workspace', path: 'C:/workspace' }], tasks: { one: { projectId: 'project' }, two: { projectId: 'project' } } };
    mocks.workspaceIndex.mockResolvedValue(index);
    let finishCreate!: (value: { id: string; title: string; updatedAt: number }) => void;
    mocks.createConversation.mockImplementationOnce(() => new Promise(resolve => { finishCreate = resolve; }));
    render(<App />);
    await screen.findByRole('button', { name: 'Research notes' });
    fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), { target: { value: 'Start a new task' } });
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(finishCreate).toBeTypeOf('function'));
    const other = screen.getByRole('button', { name: 'Research notes' });
    expect(other).toBeEnabled();
    await userEvent.click(other);
    await screen.findByText('Research notes context');
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'Draft in the other conversation');
    await act(async () => finishCreate({ id: 'created', title: 'New task', updatedAt: 3 }));
    await waitFor(() => expect(f.channels.has('created')).toBe(true));
    expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Draft in the other conversation');
    expect(screen.getByText('Research notes context')).toBeInTheDocument();
    await act(async () => f.emit('created', 'Background work'));
    expect(screen.queryByText('Background work')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(f.channels.has('one')).toBe(true));
    await act(async () => f.emit('one', 'Other session work'));
    expect(screen.getByText('Other session work')).toBeInTheDocument();
    expect(screen.queryByText('Background work')).not.toBeInTheDocument();
    await act(async () => f.finish.get('created')!());
    await act(async () => f.finish.get('one')!());
  });

  it('keeps streams, drafts, completion and Stop attached to their conversation', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    await f.submit('Research this topic');
    await act(async () => f.emit('one', 'First session is investigating.'));
    expect(screen.getByRole('button', { name: 'Draft document' })).toBeEnabled();
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await screen.findByText('Draft document context');
    expect(screen.queryByText('First session is investigating.')).not.toBeInTheDocument();
    await f.submit('Draft the report');
    await waitFor(() => expect(f.channels.has('two')).toBe(true));
    await act(async () => { f.emit('one', ' New evidence.'); f.emit('two', 'Second session draft.'); });
    expect(screen.getByText('Second session draft.')).toBeInTheDocument();
    expect(screen.queryByText(/First session is investigating/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Stop response' }));
    expect(mocks.cancelGeneration).toHaveBeenCalledWith('two');
    expect(mocks.cancelGeneration).not.toHaveBeenCalledWith('one');
    await userEvent.click(screen.getByRole('button', { name: 'Research notes' }));
    expect(await screen.findByText('First session is investigating. New evidence.')).toBeInTheDocument();
    f.history.two.push({ id: 'two-final', conversationId: 'two', role: 'assistant', content: 'Document completed elsewhere.', reasoning: '', status: 'complete', createdAt: 2 });
    await act(async () => f.finish.get('two')!());
    expect(screen.queryByText('Document completed elsewhere.')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Draft document' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop response' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    expect(await screen.findByText('Document completed elsewhere.')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'Follow up on the document');
    await userEvent.click(screen.getByRole('button', { name: 'Research notes' }));
    expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('');
    await act(async () => f.finish.get('one')!());
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Follow up on the document'));
  });

  it('does not let recovery polling overwrite a stream owned by this window', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    await f.submit('Research this topic');
    await act(async () => f.emit('one', 'Live output that is not saved yet.'));
    mocks.listActiveRuns.mockResolvedValue([{ conversationId: 'one', instanceId: 'local', phase: 'running', cancellationRequested: false,
      state: 'generating', activity: 'Streaming', approvals: [] }]);
    await waitFor(() => expect(mocks.listActiveRuns.mock.calls.length).toBeGreaterThan(1), { timeout: 6500 });
    expect(screen.getByText('Live output that is not saved yet.')).toBeInTheDocument();
    mocks.listActiveRuns.mockResolvedValue([]);
    await act(async () => f.finish.get('one')!());
  }, 10000);

  it('renders journal-backed reasoning received on the live channel before a journal poll', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    await f.submit('Research this topic');
    const base: SessionEvent = { id: 'typed-start', conversationId: 'one', runId: 'run-one', seq: 1,
      eventType: 'item.started', payload: { item: { id: 'thought-one', runId: 'run-one', round: 0, kind: 'reasoning', status: 'running', startedAt: Date.now(), payload: {} } },
      createdAt: Date.now() + 1, ignorable: false };
    await act(async () => {
      f.channels.get('one')!({ messageId: '', content: '', reasoning: '', item: base });
      f.channels.get('one')!({ messageId: '', content: '', reasoning: '', item: { ...base, id: 'typed-delta', seq: 2, eventType: 'item.delta', payload: { itemId: 'thought-one', delta: { text: 'Checking the evidence.' } } } });
      f.channels.get('one')!({ messageId: 'one-answer', content: '', reasoning: 'Checking the evidence.' });
    });
    const reasoning = screen.getByLabelText('Reasoning in progress');
    expect(reasoning).toBeInTheDocument();
    fireEvent.click(reasoning);
    expect(screen.getByText('Checking the evidence.')).toBeInTheDocument();
    await act(async () => f.finish.get('one')!());
  });

  it('shows a queued session and restores its prompt when stopped without affecting another chat', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    await f.submit('Queued research request');
    await act(async () => f.channels.get('one')!({ messageId: '', content: '', reasoning: '', state: 'queued', activity: 'Waiting for a response slot…' }));
    expect(screen.getByRole('button', { name: 'Research notes' })).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Waiting for a response slot');
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await screen.findByText('Draft document context');
    await f.submit('Continue drafting');
    await waitFor(() => expect(f.channels.has('two')).toBe(true));
    await userEvent.click(screen.getByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    fireEvent.click(screen.getByRole('button', { name: 'Stop response' }));
    expect(mocks.cancelGeneration).toHaveBeenCalledWith('one');
    expect(mocks.cancelGeneration).not.toHaveBeenCalledWith('two');
    await act(async () => f.reject.get('one')!(new Error('Message cancelled before generation; it was not saved.')));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Queued research request'));
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await act(async () => f.emit('two', 'Still working.'));
    expect(screen.getByText('Still working.')).toBeInTheDocument();
    await act(async () => f.finish.get('two')!());
  });

  it('offers an unsent saved request for explicit recovery without replaying it on open', async () => {
    const f = fixture();
    const saved = { id: 'pending-one', conversationId: 'one', content: 'Saved queued request', createdAt: 1 };
    const bootstrap = await mocks.bootstrap();
    mocks.bootstrap.mockResolvedValue({ ...bootstrap, pendingConversationIds: ['one'] });
    mocks.getPendingTurn.mockImplementation(async (id: string) => id === 'one' ? saved : null);
    render(<App />);
    expect(await screen.findByText('Unsent request')).toBeVisible();
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    expect(await screen.findByRole('region', { name: 'Saved unsent request' })).toBeVisible();
    expect(screen.getByRole('button', { name: 'Research notes' })).toBeVisible();
    expect(mocks.sendMessage).not.toHaveBeenCalled();
    await userEvent.click(screen.getByText('Review request'));
    expect(screen.getByText('Saved queued request')).toBeVisible();
    mocks.getPendingTurn.mockResolvedValue(null);
    await userEvent.click(screen.getByRole('button', { name: 'Send saved request' }));
    await waitFor(() => expect(f.channels.has('one')).toBe(true));
    const call = mocks.sendMessage.mock.calls[mocks.sendMessage.mock.calls.length - 1];
    expect(call[1]).toBe('Saved queued request');
    expect(call[8]).toBe('pending-one');
    await act(async () => f.finish.get('one')!());
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Saved unsent request' })).not.toBeInTheDocument());
  });

  it('requires an explicit discard of an unsent saved request', async () => {
    fixture();
    mocks.getPendingTurn.mockResolvedValue({ id: 'saved-id', conversationId: 'one', content: 'Saved request', createdAt: 1 });
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByRole('region', { name: 'Saved unsent request' });
    mocks.confirm.mockResolvedValueOnce(false);
    await userEvent.click(screen.getByRole('button', { name: 'Discard saved request' }));
    expect(mocks.discardPendingTurn).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Discard saved request' }));
    await waitFor(() => expect(mocks.discardPendingTurn).toHaveBeenCalledWith('one', 'saved-id'));
    expect(screen.queryByRole('region', { name: 'Saved unsent request' })).not.toBeInTheDocument();
  });

  it('recovers the exact saved image-attachment payload', async () => {
    const f = fixture();
    const content = JSON.stringify({ kind: 'locallm-attachments-v1', text: 'Review this', attachments: [{ name: 'screen.png', content: '', imageUrl: 'data:image/png;base64,AA==' }] });
    mocks.getPendingTurn.mockResolvedValue({ id: 'image-request', conversationId: 'one', content, createdAt: 1 });
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByRole('region', { name: 'Saved unsent request' });
    await userEvent.click(screen.getByText('Review request'));
    await userEvent.click(screen.getByText('screen.png'));
    expect(screen.getByRole('img', { name: 'screen.png' })).toBeVisible();
    mocks.getPendingTurn.mockResolvedValue(null);
    await userEvent.click(screen.getByRole('button', { name: 'Send saved request' }));
    await waitFor(() => expect(f.channels.has('one')).toBe(true));
    const call = mocks.sendMessage.mock.calls[mocks.sendMessage.mock.calls.length - 1];
    expect(call[1]).toBe(content);
    expect(call[8]).toBe('image-request');
    await act(async () => f.finish.get('one')!());
  });

  it('restores a rejected hidden prompt to its original draft', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context');
    await f.submit('Keep this rejected research prompt');
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await screen.findByText('Draft document context');
    await userEvent.type(screen.getByRole('textbox', { name: 'Message' }), 'Keep this document draft');
    await act(async () => f.reject.get('one')!(new Error('Request rejected before persistence')));
    expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Keep this document draft');
    expect(screen.queryByText('Request rejected before persistence')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Research notes' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Message' })).toHaveValue('Keep this rejected research prompt'));
    expect(screen.getByRole('alert')).toHaveTextContent('Request rejected before persistence');
  });

  it('ignores a delayed completion snapshot after a newer session refresh', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context'); await f.submit('Research');
    await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
    await screen.findByText('Draft document context'); await f.submit('Write');
    await waitFor(() => expect(f.channels.size).toBe(2));
    const oldSnapshot = await mocks.bootstrap();
    let resolveOld!: (value: typeof oldSnapshot) => void;
    mocks.bootstrap.mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; }));
    mocks.bootstrap.mockResolvedValueOnce({ ...oldSnapshot, conversations: oldSnapshot.conversations.map((item: { id: string }) => item.id === 'two' ? { ...item, title: 'Updated document' } : item) });
    await act(async () => f.finish.get('one')!());
    await waitFor(() => expect(resolveOld).toBeTypeOf('function'));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Research notes' })).toBeInTheDocument());
    await act(async () => f.finish.get('two')!());
    expect(await screen.findByRole('button', { name: 'Updated document' })).toBeInTheDocument();
    await act(async () => resolveOld(oldSnapshot));
    expect(screen.getByRole('button', { name: 'Updated document' })).toBeInTheDocument();
  });

  it('keeps a saved answer when the conversation list cannot refresh', async () => {
    const f = fixture();
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
    await screen.findByText('Research notes context'); await f.submit('Research');
    f.history.one.push({ id: 'saved-answer', conversationId: 'one', role: 'assistant', content: 'Saved answer', reasoning: '', status: 'complete', createdAt: 2 });
    mocks.bootstrap.mockRejectedValueOnce(new Error('List unavailable'));
    await act(async () => f.finish.get('one')!());
    expect(await screen.findByText('Saved answer')).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('Could not refresh conversations: List unavailable');
  });

  it('queues approvals across sessions and closes only the named request', async () => {
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.open = true; } });
    try {
      const f = fixture();
      render(<App />);
      await userEvent.click(await screen.findByRole('button', { name: 'Research notes' }));
      await screen.findByText('Research notes context'); await f.submit('Research');
      await userEvent.click(screen.getByRole('button', { name: 'Draft document' }));
      await screen.findByText('Draft document context'); await f.submit('Write');
      await waitFor(() => expect(f.channels.size).toBe(2));
      const approval = (id: string) => ({ id, connector: 'Workspace', name: 'write_file', arguments: { path: 'report.md' } });
      // The later-created session asks first. A subsequent request from the
      // earlier session must not replace a form the user is already reading.
      await act(async () => f.channels.get('two')!({ messageId: '', content: '', reasoning: '', approval: approval('approve-two') }));
      expect(screen.getByText('Conversation: Draft document')).toBeInTheDocument();
      await act(async () => f.channels.get('one')!({ messageId: '', content: '', reasoning: '', approval: approval('approve-one') }));
      expect(screen.getByText('Conversation: Draft document')).toBeInTheDocument();
      await act(async () => f.channels.get('two')!({ messageId: '', content: '', reasoning: '', approval: { ...approval('approve-two'), closed: true } }));
      expect(screen.getByText('Conversation: Research notes')).toBeInTheDocument();
      await act(async () => f.channels.get('two')!({ messageId: '', content: '', reasoning: '', approval: approval('approve-two-next') }));
      fireEvent.click(screen.getByRole('button', { name: 'Allow once' }));
      await waitFor(() => expect(mocks.resolveToolApproval).toHaveBeenCalledWith('approve-one', true));
      await act(async () => f.channels.get('one')!({ messageId: '', content: '', reasoning: '', approval: { ...approval('approve-one'), closed: true } }));
      expect(screen.getByText('Conversation: Draft document')).toBeInTheDocument();
      // A delayed close for the previous session cannot dismiss this dialog.
      await act(async () => f.channels.get('one')!({ messageId: '', content: '', reasoning: '', approval: { ...approval('approve-one'), closed: true } }));
      expect(screen.getByText('Conversation: Draft document')).toBeInTheDocument();
      await act(async () => { f.finish.get('one')!(); f.finish.get('two')!(); });
    } finally {
      if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal');
    }
  });
});

it('serializes workspace navigation and ignores a superseded project selection', async () => {
  saveDraft('one', ''); saveDraft('two', '');
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, conversations: [{ id: 'one', title: 'First task', updatedAt: 1 }, { id: 'two', title: 'Second task', updatedAt: 2 }], rememberedTools: { sources: [], tools: [] } });
  mocks.workspaceIndex.mockResolvedValue({ projects: [{ id: 'p-one', name: 'First project', path: 'C:/first' }, { id: 'p-two', name: 'Second project', path: 'C:/second' }], tasks: { one: { projectId: 'p-one' }, two: { projectId: 'p-two' } } });
  mocks.messages.mockImplementation(async (id: string) => [{ id: `msg-${id}`, conversationId: id, role: 'user', content: `Content for ${id}`, reasoning: '', status: 'complete', createdAt: 1 }]);
  let releaseFirst!: () => void;
  mocks.setWorkspace.mockImplementationOnce(() => new Promise<void>(resolve => { releaseFirst = resolve; })).mockResolvedValue(undefined);
  render(<App />);
  await userEvent.click(await screen.findByRole('button', { name: 'First task' }));
  await waitFor(() => expect(mocks.setWorkspace).toHaveBeenCalledWith('C:/first'));
  await userEvent.click(screen.getByRole('button', { name: 'Second task' }));
  expect(mocks.setWorkspace).toHaveBeenCalledTimes(1);
  await act(async () => releaseFirst());
  expect(await screen.findByText('Content for two')).toBeInTheDocument();
  expect(screen.getByRole('combobox', { name: 'Task project' })).toHaveValue('p-two');
  expect(mocks.setWorkspace).toHaveBeenLastCalledWith('C:/second');
  expect(mocks.messages).not.toHaveBeenCalledWith('one');
  expect(screen.queryByText('Content for one')).not.toBeInTheDocument();
});
