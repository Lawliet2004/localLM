import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { defaultRuntimeConfig } from './lib/types';
const mocks = vi.hoisted(() => ({
  save: vi.fn(), exportConversation: vi.fn(), bootstrap: vi.fn(),
  saveRememberedTools: vi.fn(), saveConversationTools: vi.fn(), conversationTools: vi.fn(),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save, open: vi.fn(), confirm: vi.fn() }));
vi.mock('./lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
  bootstrap: mocks.bootstrap, exportConversation: mocks.exportConversation,
  saveRememberedTools: mocks.saveRememberedTools, saveConversationTools: mocks.saveConversationTools, conversationTools: mocks.conversationTools,
  listConnectors: async () => [{ id: 'example', description: 'Example service', url: '', authType: 'apiKey', connected: true, hasCredential: true, tools: [{ name: 'kept_tool', description: 'Kept for new chats', inputSchema: {} }] }],
  listSkills: async () => [], getWorkspace: async () => ({ path: 'C:/workspace' }), hasDaytonaKey: async () => false,
  listPresets: async () => [], getPreset: async () => ({ id: 'standard', name: 'Standard', description: '', sources: [], mcp: true, systemTime: true, skills: true, harness: [] }),
  messages: async () => [{ id: 'message', conversationId: 'chat', role: 'user', content: 'Export this message', reasoning: '', status: 'complete', createdAt: 0 }],
} }));
const baseBootstrap = { conversations: [{ id: 'chat', title: 'Saved chat', updatedAt: 0 }], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 100, systemPrompt: '' }, runtime: { phase: 'stopped', message: '', modelPath: null },
  providers: [], preferredModel: { providerId: null, modelId: '' } };
beforeEach(() => {
  vi.clearAllMocks();
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
  it('shows full access clearly and resets a new conversation to asking', async () => {
    render(<App />);
    await screen.findByRole('button', { name: 'Saved chat' });
    const mode = screen.getByRole('combobox', { name: 'Permission mode' });
    expect(mode).toHaveValue('ask');
    await userEvent.selectOptions(mode, 'fullAccess');
    expect(mode).toHaveValue('fullAccess');
    expect(screen.getByText('Selected tools run without prompts, including code and external changes.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    expect(mode).toHaveValue('ask');
    await userEvent.selectOptions(mode, 'autoApprove');
    expect(screen.getByText('Workspace reads run automatically. Other actions ask.')).toBeInTheDocument();
  });
});

describe('remembered tool selection', () => {
  it('restores the remembered selection for a new chat at startup', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: ['__workspace'], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    render(<App />);
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeChecked());
    // The picker must not flash the Off state before the saved load completes.
    expect(screen.getByText('Tools · 6/32 enabled')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('saves a change in a fresh chat immediately as the remembered selection', async () => {
    render(<App />);
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeEnabled());
    await userEvent.click(workspace);
    await waitFor(() => expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: ['__workspace'], tools: [] }));
    expect(screen.getByText('Tools · 5/32 enabled')).toBeInTheDocument();
    // No conversation exists before the first message, so nothing is written per conversation.
    expect(mocks.saveConversationTools).not.toHaveBeenCalled();
  });
  it('reports a failed save without claiming the choice was applied', async () => {
    mocks.saveRememberedTools.mockRejectedValue(new Error('The disk is full.'));
    render(<App />);
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
    await screen.findByText('Tools · 1/32 enabled');
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    expect(await screen.findByText('Tools · 1/32 enabled')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('existing conversations keep their own selections and leave the remembered one alone', async () => {
    mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, rememberedTools: { sources: [], tools: [{ connectorId: 'example', toolName: 'kept_tool' }] } });
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await screen.findByText('Tools · 1/32 enabled');
    await userEvent.click(screen.getByRole('button', { name: 'Saved chat' }));
    const workspace = await screen.findByRole('checkbox', { name: 'Workspace files' });
    await waitFor(() => expect(workspace).toBeChecked());
    expect(screen.getByText('Tools · 5/32 enabled')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
    // Opening another chat never changes the future-chat default.
    await userEvent.click(screen.getByRole('button', { name: /New conversation/ }));
    expect(await screen.findByText('Tools · 1/32 enabled')).toBeInTheDocument();
    expect(mocks.saveRememberedTools).not.toHaveBeenCalled();
  });
  it('changing tools in a saved conversation updates it and the remembered selection', async () => {
    mocks.conversationTools.mockResolvedValue({ sources: ['__workspace'], tools: [], accessMode: 'ask' });
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Saved chat' }));
    await screen.findByText('Tools · 5/32 enabled');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Workspace files' }));
    await waitFor(() => expect(mocks.saveConversationTools).toHaveBeenCalledWith('chat', { sources: [], tools: [], accessMode: 'ask' }));
    expect(mocks.saveRememberedTools).toHaveBeenCalledWith({ sources: [], tools: [] });
    expect(screen.getByText('Tools · Off')).toBeInTheDocument();
  });
});
