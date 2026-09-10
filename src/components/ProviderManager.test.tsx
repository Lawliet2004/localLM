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
