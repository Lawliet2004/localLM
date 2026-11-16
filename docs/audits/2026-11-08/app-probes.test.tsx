// Diagnostic fixture setup copied from src/App.test.tsx; production App is imported directly.
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../../../src/App';
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: async () => false, onResized: async () => () => {} }) }));
import type { ActiveRunSnapshot, ChatEvent, Message, SessionEvent } from '../../../src/lib/types';
import { toolOutput } from '../../../src/lib/toolOutput';
import { saveDraft } from '../../../src/lib/drafts';
import { defaultRuntimeConfig } from '../../../src/lib/types';
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
vi.mock('../../../src/lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
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

it('diagnostic: reused call ids overwrite the prior live operation and join their output', async () => {
  mocks.bootstrap.mockResolvedValue({ ...baseBootstrap, runtime: { phase: 'ready', message: '', modelPath: 'fixture.gguf' }, rememberedTools: { sources: [], tools: [] } });
  let receive!: (event: ChatEvent) => void;
  let finish!: () => void;
  mocks.sendMessage.mockImplementation((_id: string, _content: string, callback: (event: ChatEvent) => void) => {
    receive = callback;
    return new Promise<void>(resolve => { finish = resolve; });
  });
  await openChat();
  fireEvent.change(screen.getByRole('textbox', { name: 'Message' }), { target: { value: 'Run two independent commands' } });
  fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
  await waitFor(() => expect(mocks.sendMessage).toHaveBeenCalledOnce());
  const emit = (stepId: string, toolStream: NonNullable<ChatEvent['toolStream']>) => receive({ runId: 'audit-run', stepId, messageId: '', content: '', reasoning: '', toolStream });
  await act(async () => {
    emit('step-0', { type: 'started', toolCallId: 'textcall_0', toolName: 'execute_command', command: 'printf first-command' });
    emit('step-0', { type: 'outputChunk', toolCallId: 'textcall_0', stream: 'stdout', chunk: 'first-result\n' });
    emit('step-0', { type: 'finished', toolCallId: 'textcall_0', exitCode: 0, durationMs: 1 });
  });
  await act(async () => {
    emit('step-1', { type: 'started', toolCallId: 'textcall_0', toolName: 'execute_command', command: 'printf second-command' });
    emit('step-1', { type: 'outputChunk', toolCallId: 'textcall_0', stream: 'stdout', chunk: 'second-result\n' });
  });
  expect(screen.getAllByTestId('command-run-card')).toHaveLength(1);
  expect(screen.getByTestId('command-run-card')).toHaveTextContent('printf first-command');
  expect(screen.getByTestId('command-run-card')).not.toHaveTextContent('printf second-command');
  expect(toolOutput.read('chat', 'tool-textcall_0').stdout).toBe('first-result\nsecond-result\n');
  await act(async () => emit('step-1', { type: 'finished', toolCallId: 'textcall_0', exitCode: 0, durationMs: 2 }));
  const card = screen.getByTestId('command-run-card');
  if (!card.hasAttribute('open')) fireEvent.click(card.querySelector('summary')!);
  expect(screen.getByTestId('terminal-viewport')).toHaveTextContent(/first-result.*second-result/s);
  await act(async () => finish());
});

