import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ProviderManager } from './ProviderManager';
import { api } from '../lib/api';
import type { ProviderConnection } from '../lib/types';

vi.mock('../lib/api', () => ({
  errorMessage: (error: unknown) => String(error),
  api: {
    saveProvider: vi.fn(), deleteProvider: vi.fn(), testProvider: vi.fn(), listProviderModels: vi.fn(),
    getSubscriptionStatus: vi.fn().mockResolvedValue({ provider: '', connected: false, isExpired: false }),
    detectSubscriptionCli: vi.fn().mockResolvedValue({ provider: '', found: false }),
    importSubscriptionCli: vi.fn(),
    saveManualSubscriptionToken: vi.fn(),
    disconnectSubscription: vi.fn(),
    startSubscriptionSignIn: vi.fn(),
    cancelSubscriptionSignIn: vi.fn(),
  },
}));

const provider: ProviderConnection = {
  id: 'provider-1', name: 'Saved provider', apiFormat: 'openai-chat-completions',
  baseUrl: 'https://api.example.test/v1', verified: false, lastTestedAt: null, hasApiKey: true,
  models: [{ id: 'remote-model', contextLength: 32768, maxOutputTokens: null, toolSupport: 'unknown' }],
};

it('never renders the saved API key and supports an explicit connection test', async () => {
  vi.mocked(api.testProvider).mockResolvedValue({ verified: true, modelListSupported: true, models: ['remote-model'], message: 'Connection verified.' });
  const onChanged = vi.fn().mockResolvedValue(undefined);
  render(<ProviderManager providers={[provider]} busy={false} onChanged={onChanged} />);

  expect(screen.queryByText(/secret|key-value/)).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));

  await waitFor(() => expect(api.testProvider).toHaveBeenCalledExactlyOnceWith(provider.id));
  expect(await screen.findByText('Connection verified.')).toBeInTheDocument();
  expect(onChanged).toHaveBeenCalled();
});

it('adds a manually configured model with explicit limits', async () => {
  const onChanged = vi.fn().mockResolvedValue(undefined);
  render(<ProviderManager providers={[provider]} busy={false} onChanged={onChanged} />);

  await userEvent.type(screen.getByRole('textbox', { name: 'Manual model ID' }), 'manual-model');
  await userEvent.click(screen.getByRole('button', { name: 'Add model' }));
  expect(screen.getByDisplayValue('manual-model')).toBeInTheDocument();
  expect(screen.getByText(/Context capacity is required for chat/)).toBeInTheDocument();
});

it('requires saving edited connection fields before testing the saved provider', async () => {
  render(<ProviderManager providers={[provider]} busy={false} onChanged={vi.fn()} />);
  await userEvent.type(screen.getByLabelText('Provider base URL'), '/custom');
  expect(screen.getByRole('button', { name: 'Test connection' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Refresh models' })).toBeDisabled();
  expect(screen.getByText('Save changes before testing the connection or refreshing models.')).toBeVisible();
});

it('displays detected CLI login and supports one-click sync', async () => {
  vi.mocked(api.detectSubscriptionCli).mockImplementation(async (providerName) => {
    if (providerName === 'chatgpt') {
      return { provider: 'chatgpt', found: true, cliPath: 'C:\\Users\\test\\.codex\\auth.json', accountEmail: 'test@example.com' };
    }
    return { provider: 'grok', found: false };
  });
  vi.mocked(api.importSubscriptionCli).mockResolvedValue({
    provider: 'chatgpt',
    connected: true,
    accountEmail: 'test@example.com',
    planType: 'ChatGPT Plus',
    isExpired: false,
  });
  const onChanged = vi.fn().mockResolvedValue(undefined);
  render(<ProviderManager providers={[provider]} busy={false} onChanged={onChanged} />);

  expect(await screen.findByText(/Found CLI credentials: C:\\Users\\test\\\.codex\\auth\.json/)).toBeInTheDocument();
  const syncBtn = screen.getByRole('button', { name: 'Sync from CLI' });
  await userEvent.click(syncBtn);

  expect(api.importSubscriptionCli).toHaveBeenCalledWith('chatgpt');
  expect(await screen.findByText(/Imported ChatGPT credentials from CLI successfully/)).toBeInTheDocument();
  expect(onChanged).toHaveBeenCalled();
});

it('supports saving manual subscription token', async () => {
  vi.mocked(api.saveManualSubscriptionToken).mockResolvedValue({
    provider: 'grok',
    connected: true,
    accountEmail: null,
    planType: 'SuperGrok',
    isExpired: false,
  });
  const onChanged = vi.fn().mockResolvedValue(undefined);
  render(<ProviderManager providers={[provider]} busy={false} onChanged={onChanged} />);

  const manualBtns = screen.getAllByRole('button', { name: 'Manual token' });
  await userEvent.click(manualBtns[1]); // Grok manual token button

  const tokenInput = await screen.findByLabelText('Grok Access Token');
  await userEvent.type(tokenInput, 'eyJhbGciOiJIUzI1NiJ9.test');
  await userEvent.click(screen.getByRole('button', { name: 'Save' }));

  expect(api.saveManualSubscriptionToken).toHaveBeenCalledWith('grok', 'eyJhbGciOiJIUzI1NiJ9.test', null, null);
  expect(await screen.findByText(/Saved Grok token successfully/)).toBeInTheDocument();
  expect(onChanged).toHaveBeenCalled();
});

it('enables testing connection for loopback providers without an API key', async () => {
  vi.mocked(api.testProvider).mockResolvedValue({ verified: true, modelListSupported: true, models: ['local-q4'], message: 'Loopback OK' });
  const loopbackProvider: ProviderConnection = {
    id: 'ollama', name: 'Ollama Local', apiFormat: 'openai-chat-completions',
    baseUrl: 'http://127.0.0.1:11434/v1', verified: false, lastTestedAt: null, hasApiKey: false,
    models: [{ id: 'local-q4', contextLength: 4096, maxOutputTokens: null, toolSupport: 'unknown' }],
  };
  render(<ProviderManager providers={[loopbackProvider]} busy={false} onChanged={vi.fn()} />);

  const testBtn = screen.getByRole('button', { name: 'Test connection' });
  expect(testBtn).toBeEnabled();
  await userEvent.click(testBtn);
  expect(api.testProvider).toHaveBeenCalledWith('ollama');
  expect(await screen.findByText('Loopback OK')).toBeInTheDocument();
});

