import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ModelSelectorPanel } from './ModelSelectorPanel';
import type { ProviderConnection } from '../lib/types';

const provider: ProviderConnection = {
  id: 'provider-1', name: 'OpenAI-compatible', apiFormat: 'openai-chat-completions',
  baseUrl: 'https://api.example.test/v1', verified: true, lastTestedAt: 1, hasApiKey: true,
  models: [{ id: 'remote-model', contextLength: 32768, maxOutputTokens: 4096, toolSupport: 'supported' }],
};

it('saves a remote provider and model selection without exposing the key', async () => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(<ModelSelectorPanel providers={[provider]} selection={{ providerId: null, modelId: '' }} busy={false} onSave={onSave} />);

  await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Inference provider' }), provider.id);
  await userEvent.type(screen.getByRole('combobox', { name: 'Remote model ID' }), 'remote-model');
  expect(screen.getByText(/Remote usage may incur provider charges/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Save selection' }));

  expect(onSave).toHaveBeenCalledExactlyOnceWith({ providerId: provider.id, modelId: 'remote-model' });
});

it('allows an explicit return to local inference', async () => {
  const onSave = vi.fn().mockResolvedValue(undefined);
  render(<ModelSelectorPanel providers={[provider]} selection={{ providerId: provider.id, modelId: 'remote-model' }} busy={false} onSave={onSave} />);

  await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Inference provider' }), '');
  await userEvent.click(screen.getByRole('button', { name: 'Save selection' }));

  expect(onSave).toHaveBeenCalledExactlyOnceWith({ providerId: null, modelId: '' });
});
