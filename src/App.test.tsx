import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import { defaultRuntimeConfig } from './lib/types';
const mocks = vi.hoisted(() => ({ save: vi.fn(), exportConversation: vi.fn(), bootstrap: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.save, open: vi.fn(), confirm: vi.fn() }));
vi.mock('./lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: {
  bootstrap: mocks.bootstrap, exportConversation: mocks.exportConversation,
  listConnectors: async () => [], listSkills: async () => [], getWorkspace: async () => ({ path: '' }),
  conversationTools: async () => ({ sources: [], tools: [] }),
  messages: async () => [{ id: 'message', conversationId: 'chat', role: 'user', content: 'Export this message', reasoning: '', status: 'complete', createdAt: 0 }],
} }));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.bootstrap.mockResolvedValue({ conversations: [{ id: 'chat', title: 'Saved chat', updatedAt: 0 }], config: defaultRuntimeConfig,
    preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 100, systemPrompt: '' }, runtime: { phase: 'stopped', message: '', modelPath: null } });
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
