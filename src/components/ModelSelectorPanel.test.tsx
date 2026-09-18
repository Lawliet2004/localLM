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
it('offers the exact Bonsai GGUF and opens its setup without changing the remote selection', async () => {
  const onLocalModelChange = vi.fn();
  const onSave = vi.fn();
  render(<ModelSelectorPanel providers={[]} selection={{providerId:null,modelId:''}} busy={false} onSave={onSave} onLocalModelChange={onLocalModelChange} />);
  await userEvent.selectOptions(screen.getByRole('combobox', {name:'Local model option'}), 'Ternary-Bonsai-8B-Q2_0.gguf');
  expect(onLocalModelChange).toHaveBeenCalledWith('Ternary-Bonsai-8B-Q2_0.gguf');
  expect(onSave).not.toHaveBeenCalled();
});

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

it('displays subscription badge and quota explanation for subscription providers', async () => {
  const subProvider: ProviderConnection = {
    id: 'chatgpt-subscription',
    name: 'ChatGPT Subscription',
    apiFormat: 'chatgpt-subscription',
    baseUrl: 'https://chatgpt.com/backend-api/latents',
    verified: true,
    lastTestedAt: 1,
    hasApiKey: true,
    models: [{ id: 'chatgpt-4o-latest', contextLength: 128000, maxOutputTokens: 16384, toolSupport: 'supported' }],
  };
  render(<ModelSelectorPanel providers={[subProvider]} selection={{ providerId: subProvider.id, modelId: 'chatgpt-4o-latest' }} busy={false} onSave={vi.fn()} />);

  expect(screen.getByText('Subscription (Quota included)')).toBeInTheDocument();
  expect(screen.getByText(/included monthly quota/)).toBeInTheDocument();
});

