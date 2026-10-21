import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { Providers } from './Providers';
import type { ModelControls, ProviderConnection } from '../lib/types';

const env = vi.hoisted(() => ({ native: true }));
const apiMocks = vi.hoisted(() => ({
  saveProvider: vi.fn(),
  testProvider: vi.fn(),
  testProviderDraft: vi.fn(),
  listProviderModelsDraft: vi.fn(),
  deleteProvider: vi.fn(),
  deleteProviderApiKey: vi.fn(),
  listProviderModels: vi.fn(),
  testProviderInference: vi.fn(),
  probeModelTools: vi.fn(),
}));
vi.mock('../lib/api', () => ({
  api: apiMocks,
  errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e),
  get nativeAvailable() { return env.native; },
}));

const verified: ProviderConnection = {
  id: 'prov-1', name: 'Acme', apiFormat: 'openai-chat-completions',
  baseUrl: 'https://api.example.com/v1', verified: true, lastTestedAt: 1_700_000_000_000,
  hasApiKey: true,
  models: [{ id: 'acme-1', contextLength: 8192, maxOutputTokens: 1024, toolSupport: 'supported', supportsImages: false }],
};
const loopback: ProviderConnection = {
  ...verified, id: 'local-1', name: 'Local engine', baseUrl: 'http://127.0.0.1:8080',
  verified: false, lastTestedAt: null, hasApiKey: false,
};
const subscription: ProviderConnection = {
  ...verified, id: 'chatgpt-subscription', name: 'ChatGPT (Subscription)',
  apiFormat: 'chatgpt-subscription', baseUrl: 'https://chatgpt.com/backend-api/codex',
};

function renderProviders(providers: ProviderConnection[] = [], selection = { providerId: null as string | null, modelId: '' }) {
  const onSaveSelection = vi.fn().mockResolvedValue(undefined);
  const onRefresh = vi.fn().mockResolvedValue(undefined);
  render(<Providers providers={providers} selection={selection} busy={false} onSaveSelection={onSaveSelection} onRefresh={onRefresh} />);
  return { onSaveSelection, onRefresh };
}
async function openEditor() {
  await userEvent.click(screen.getAllByRole('button', { name: 'Add provider' })[0]);
}
async function fillNewProvider(key = 'sk-test') {
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Acme');
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://api.example.com/v1');
  if (key) await userEvent.type(screen.getByLabelText('API key'), key);
}

beforeEach(() => {
  env.native = true;
  vi.clearAllMocks();
});

it('shows a thoughtful empty state and opens the add editor', async () => {
  renderProviders([]);
  expect(screen.getByText('No API providers yet')).toBeInTheDocument();
  await openEditor();
  expect(screen.getByLabelText('Provider name')).toBeInTheDocument();
  expect(screen.getByLabelText('API base URL')).toBeInTheDocument();
  expect(screen.getByLabelText('API key')).toBeInTheDocument();
  expect(screen.getByLabelText('API format')).toHaveValue('openai-chat-completions');
});

it('lets the browser preview explore the editor while save and test stay disabled', async () => {
  env.native = false;
  renderProviders([]);
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Acme');
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://api.example.com/v1');
  expect(screen.getByText(/Browser preview/)).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Test connection' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Save provider' })).toBeDisabled();
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
});

it('tests a new provider draft through native IPC without persisting it', async () => {
  apiMocks.testProviderDraft.mockResolvedValue({ verified: true, modelListSupported: true, models: ['acme-1'], message: 'Connection verified. Model listing returned 1 model.' });
  renderProviders([]);
  await fillNewProvider();
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  await vi.waitFor(() => expect(apiMocks.testProviderDraft).toHaveBeenCalledTimes(1));
  expect(apiMocks.testProviderDraft.mock.calls[0][0]).toMatchObject({
    id: null, name: 'Acme', apiFormat: 'openai-chat-completions',
    baseUrl: 'https://api.example.com/v1', apiKey: 'sk-test', models: [],
  });
  // Testing never saves — the provider list is untouched.
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  expect(await screen.findByRole('status')).toHaveTextContent('Connection verified');
  // The discovered model ID is merged into the rows for a later Save.
  expect(screen.getByLabelText('Model 1 ID')).toHaveValue('acme-1');
});

it('shows a draft test failure inline and keeps the draft for retry', async () => {
  apiMocks.testProviderDraft
    .mockRejectedValueOnce(new Error('Provider request timed out.'))
    .mockResolvedValue({ verified: true, modelListSupported: true, models: [], message: 'Connection verified.' });
  renderProviders([]);
  await fillNewProvider();
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Provider request timed out.');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  // The editor stays open with the draft intact.
  expect(screen.getByLabelText('Provider name')).toBeInTheDocument();
  expect(screen.getByLabelText('API key')).toHaveValue('sk-test');

  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  expect(await screen.findByRole('status')).toHaveTextContent('Connection verified');
  expect(apiMocks.testProviderDraft).toHaveBeenCalledTimes(2);
});

it('edits a provider without resending the stored key', async () => {
  apiMocks.saveProvider.mockResolvedValue(verified);
  renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  const name = screen.getByLabelText('Provider name');
  await userEvent.clear(name);
  await userEvent.type(name, 'Acme renamed');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(1));
  expect(apiMocks.saveProvider.mock.calls[0][0]).toMatchObject({
    id: 'prov-1', name: 'Acme renamed', apiKey: null,
    models: [{ id: 'acme-1', contextLength: 8192, maxOutputTokens: 1024, toolSupport: 'supported', supportsImages: false }],
  });
  expect(apiMocks.testProvider).not.toHaveBeenCalled();
});

it('validates required fields and remote URL rules before saving', async () => {
  renderProviders([]);
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Acme');
  await userEvent.type(screen.getByLabelText('API base URL'), 'http://api.example.com/v1');
  await userEvent.type(screen.getByLabelText('API key'), 'sk-test');
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Remote providers require HTTPS');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  await userEvent.clear(screen.getByLabelText('API base URL'));
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://api.example.com/v1');
  await userEvent.clear(screen.getByLabelText('API key'));
  // Keyless remote providers are allowed — the key is optional for
  // OpenAI-compatible endpoints, so the draft test runs without saving.
  apiMocks.testProviderDraft.mockResolvedValue({ verified: true, modelListSupported: true, models: [], message: 'Connection verified.' });
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  await vi.waitFor(() => expect(apiMocks.testProviderDraft).toHaveBeenCalledTimes(1));
  expect(apiMocks.testProviderDraft.mock.calls[0][0]).toMatchObject({ baseUrl: 'https://api.example.com/v1', apiKey: null });
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
});

it('saves a loopback endpoint without an API key', async () => {
  apiMocks.saveProvider.mockResolvedValue(loopback);
  const { onRefresh } = renderProviders([]);
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Local engine');
  await userEvent.type(screen.getByLabelText('API base URL'), 'http://127.0.0.1:8080');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(1));
  expect(apiMocks.saveProvider.mock.calls[0][0]).toMatchObject({ baseUrl: 'http://127.0.0.1:8080', apiKey: null });
  expect(onRefresh).toHaveBeenCalled();
});

it('saves a manual model with its capacity and rejects duplicates and bad bounds', async () => {
  apiMocks.saveProvider.mockResolvedValue(verified);
  renderProviders([]);
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Acme');
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://api.example.com/v1');
  await userEvent.type(screen.getByLabelText('API key'), 'sk-test');
  await userEvent.click(screen.getByRole('button', { name: 'Add model ID' }));
  await userEvent.type(screen.getByLabelText('Model 1 ID'), 'acme-1');
  await userEvent.type(screen.getByLabelText('Model 1 context tokens'), '50');
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('between 128 and 2,000,000');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  await userEvent.clear(screen.getByLabelText('Model 1 context tokens'));
  await userEvent.type(screen.getByLabelText('Model 1 context tokens'), '8192');
  await userEvent.click(screen.getByRole('button', { name: 'Add model ID' }));
  await userEvent.type(screen.getByLabelText('Model 2 ID'), 'acme-1');
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Duplicate model ID');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  await userEvent.clear(screen.getByLabelText('Model 2 ID'));
  await userEvent.type(screen.getByLabelText('Model 2 ID'), 'acme-2');
  await userEvent.selectOptions(screen.getByLabelText('Model 1 tool support'), 'supported');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(1));
  expect(apiMocks.saveProvider.mock.calls[0][0].models).toEqual([
    { id: 'acme-1', contextLength: 8192, maxOutputTokens: null, toolSupport: 'supported', supportsImages: false },
    { id: 'acme-2', contextLength: null, maxOutputTokens: null, toolSupport: 'unknown', supportsImages: false },
  ]);
});

it('removes a provider after inline confirmation without switching an existing chat to local', async () => {
  const { onSaveSelection, onRefresh } = renderProviders([verified], { providerId: 'prov-1', modelId: 'acme-1' });
  await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
  expect(screen.getByText(/Remove Acme and its saved key/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
  await vi.waitFor(() => expect(apiMocks.deleteProvider).toHaveBeenCalledWith('prov-1'));
  expect(onSaveSelection).not.toHaveBeenCalled();
  expect(onRefresh).toHaveBeenCalled();
});

it('selects a ready remote model but never auto-selects an unverified one', async () => {
  const { onSaveSelection } = renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Use' }));
  expect(onSaveSelection).toHaveBeenCalledWith({ providerId: 'prov-1', modelId: 'acme-1' });
});

it('shows no Use action for unverified providers and offers inference tests on loopback', async () => {
  apiMocks.testProviderInference.mockResolvedValue({ verified: true, modelListSupported: false, models: ['acme-1'], message: "Inference verified: model 'acme-1' successfully completed a test request." });
  renderProviders([loopback]);
  expect(screen.queryByRole('button', { name: 'Use' })).not.toBeInTheDocument();
  expect(screen.getByText('Not verified')).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Test inference' }));
  await vi.waitFor(() => expect(apiMocks.testProviderInference).toHaveBeenCalledWith('local-1', 'acme-1'));
  expect(await screen.findByRole('status')).toHaveTextContent('Inference verified');
});

it('offers a tiny Test inference request for unverified hosted providers too', async () => {
  apiMocks.testProviderInference.mockResolvedValue({ verified: true, modelListSupported: false, models: ['acme-1'], message: "Inference verified: model 'acme-1' successfully completed a test request." });
  const { onRefresh } = renderProviders([{ ...verified, verified: false }]);
  expect(screen.queryByRole('button', { name: 'Use' })).not.toBeInTheDocument();
  const trigger = screen.getByRole('button', { name: 'Test inference' });
  expect(trigger.getAttribute('title')).toContain('tiny');
  await userEvent.click(trigger);
  await vi.waitFor(() => expect(apiMocks.testProviderInference).toHaveBeenCalledWith('prov-1', 'acme-1'));
  expect(await screen.findByRole('status')).toHaveTextContent('Inference verified');
  expect(onRefresh).toHaveBeenCalled();
});

it('retests a saved provider and refreshes even when the retest fails', async () => {
  apiMocks.testProvider.mockRejectedValue(new Error('Provider rejected the authorization or API key.'));
  const { onRefresh } = renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Test again' }));
  await vi.waitFor(() => expect(apiMocks.testProvider).toHaveBeenCalledWith('prov-1'));
  expect(await screen.findByRole('alert')).toHaveTextContent('rejected the authorization');
  expect(onRefresh).toHaveBeenCalled();
});

it('refreshes discovered models through the native list command', async () => {
  apiMocks.listProviderModels.mockResolvedValue({ ...verified, models: [...verified.models, { id: 'acme-2', contextLength: null, maxOutputTokens: null, toolSupport: 'unknown', supportsImages: false }] });
  const { onRefresh } = renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Refresh models' }));
  await vi.waitFor(() => expect(apiMocks.listProviderModels).toHaveBeenCalledWith('prov-1'));
  expect(await screen.findByRole('status')).toHaveTextContent('found 2 models');
  expect(onRefresh).toHaveBeenCalled();
});

it('renders configured subscription providers read-only and never selectable in this build', () => {
  renderProviders([subscription]);
  expect(screen.getByText('Account subscriptions')).toBeInTheDocument();
  expect(screen.getByText('ChatGPT (Subscription)')).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
  expect(screen.queryByRole('button', { name: 'Remove' })).not.toBeInTheDocument();
  // Hosted subscription inference stays gated — no Use action is offered.
  expect(screen.queryByRole('button', { name: 'Use' })).not.toBeInTheDocument();
});

it('hides the seeded subscription rows that were never configured', () => {
  renderProviders([{ ...subscription, hasApiKey: false, verified: false }]);
  expect(screen.getByText('No API providers yet')).toBeInTheDocument();
  expect(screen.queryByText('Account subscriptions')).not.toBeInTheDocument();
  expect(screen.queryByText('ChatGPT (Subscription)')).not.toBeInTheDocument();
});

it('saves an existing remote provider without a stored key', async () => {
  apiMocks.saveProvider.mockResolvedValue({ ...verified, hasApiKey: false });
  renderProviders([{ ...verified, hasApiKey: false }]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(1));
  expect(apiMocks.saveProvider.mock.calls[0][0]).toMatchObject({ id: 'prov-1', apiKey: null });
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.type(screen.getByLabelText('API key'), 'sk-restored');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(2));
  expect(apiMocks.saveProvider.mock.calls[1][0]).toMatchObject({ id: 'prov-1', apiKey: 'sk-restored' });
});

it('requires a key for Claude even on a loopback endpoint', async () => {
  renderProviders([]);
  await openEditor();
  await userEvent.type(screen.getByLabelText('Provider name'), 'Claude local');
  await userEvent.type(screen.getByLabelText('API base URL'), 'http://127.0.0.1:8080');
  await userEvent.selectOptions(screen.getByLabelText('API format'), 'claude-messages');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Claude endpoints always need an API key');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
});

it('surfaces a rejected selection save instead of an unhandled rejection', async () => {
  const onSaveSelection = vi.fn().mockRejectedValue(new Error('Store is busy.'));
  const onRefresh = vi.fn().mockResolvedValue(undefined);
  render(<Providers providers={[verified]} selection={{ providerId: null, modelId: '' }} busy={false} onSaveSelection={onSaveSelection} onRefresh={onRefresh} />);
  await userEvent.click(screen.getByRole('button', { name: 'Use' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Store is busy.');
});

it('reveals and hides the API key field', async () => {
  renderProviders([]);
  await openEditor();
  const key = screen.getByLabelText('API key');
  expect(key).toHaveAttribute('type', 'password');
  await userEvent.click(screen.getByRole('button', { name: 'Show API key' }));
  expect(key).toHaveAttribute('type', 'text');
  await userEvent.click(screen.getByRole('button', { name: 'Hide API key' }));
  expect(key).toHaveAttribute('type', 'password');
});

it('fills editable provider shortcuts and clears a draft key when the destination changes', async () => {
  renderProviders();
  await openEditor();
  await userEvent.selectOptions(screen.getByLabelText('Quick setup'), 'openrouter');
  expect(screen.getByLabelText('Provider name')).toHaveValue('OpenRouter');
  expect(screen.getByLabelText('API base URL')).toHaveValue('https://openrouter.ai/api/v1');
  await userEvent.type(screen.getByLabelText('API key'), 'router-secret');
  await userEvent.selectOptions(screen.getByLabelText('Quick setup'), 'groq');
  expect(screen.getByLabelText('API base URL')).toHaveValue('https://api.groq.com/openai/v1');
  expect(screen.getByLabelText('API key')).toHaveValue('');
  await userEvent.clear(screen.getByLabelText('API base URL'));
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://custom.example.test/prefix');
  expect(screen.getByLabelText('API base URL')).toHaveValue('https://custom.example.test/prefix');
});

it('requires an explicit replacement key after changing the endpoint of a saved provider', async () => {
  apiMocks.saveProvider.mockResolvedValue(verified);
  renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.clear(screen.getByLabelText('API base URL'));
  await userEvent.type(screen.getByLabelText('API base URL'), 'https://different.example.test/v1');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('saved key will not be sent');
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  await userEvent.type(screen.getByLabelText('API key'), 'replacement-key');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledWith(expect.objectContaining({
    id: verified.id, baseUrl: 'https://different.example.test/v1', apiKey: 'replacement-key',
  })));
});

it('deletes just the API key after confirmation and retains model selection and provider settings', async () => {
  apiMocks.deleteProviderApiKey.mockResolvedValue(undefined);
  const { onRefresh, onSaveSelection } = renderProviders([verified], { providerId: verified.id, modelId: 'acme-1' });
  await userEvent.click(screen.getByRole('button', { name: 'Delete API key' }));
  expect(apiMocks.deleteProviderApiKey).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
  expect(apiMocks.deleteProviderApiKey).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole('button', { name: 'Delete API key' }));
  await userEvent.click(screen.getByRole('button', { name: 'Delete API key' }));
  expect(apiMocks.deleteProviderApiKey).toHaveBeenCalledWith(verified.id);
  expect(apiMocks.deleteProvider).not.toHaveBeenCalled();
  expect(onSaveSelection).not.toHaveBeenCalled();
  expect(onRefresh).toHaveBeenCalled();
  expect(await screen.findByRole('status')).toHaveTextContent('API key deleted');
});

it('reports a key deletion failure and refreshes the actual connection state', async () => {
  apiMocks.deleteProviderApiKey.mockRejectedValue(new Error('Credential storage is unavailable.'));
  const { onRefresh } = renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Delete API key' }));
  await userEvent.click(screen.getByRole('button', { name: 'Delete API key' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Credential storage is unavailable');
  expect(onRefresh).toHaveBeenCalled();
});

it('can verify an already selected model again after its key has changed', async () => {
  apiMocks.testProviderInference.mockResolvedValue({ message: 'Inference verified.' });
  renderProviders([{ ...verified, verified: false }], { providerId: verified.id, modelId: 'acme-1' });
  expect(screen.getByText('Selected · unavailable')).toBeInTheDocument();
  expect(screen.queryByText('In use')).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Test inference' }));
  expect(apiMocks.testProviderInference).toHaveBeenCalledWith(verified.id, 'acme-1');
});

it('searches a large discovered catalog in the model list and editor without rendering every model', async () => {
  const models = Array.from({ length: 300 }, (_, index) => ({ ...verified.models[0], id: `model-${index}` }));
  renderProviders([{ ...verified, models }]);
  const list = screen.getByRole('list', { name: 'Acme models' });
  expect(within(list).getAllByRole('listitem')).toHaveLength(20);
  await userEvent.type(screen.getByLabelText('Find models in Acme'), 'model-299');
  expect(within(list).getAllByRole('listitem')).toHaveLength(1);
  expect(within(list).getByText('model-299')).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.type(screen.getByLabelText('Find model to edit'), 'model-299');
  expect(screen.getByLabelText('Model 300 ID')).toHaveValue('model-299');
  await userEvent.click(screen.getByRole('button', { name: 'Add model ID' }));
  expect(screen.getByLabelText('Model 301 ID')).toHaveValue('');
  await userEvent.type(screen.getByLabelText('Model 301 ID'), 'private/new-model');
  expect(screen.getByLabelText('Model 301 ID')).toHaveValue('private/new-model');
});

it('fetches the endpoint model list into the editor rows without saving', async () => {
  apiMocks.listProviderModelsDraft.mockResolvedValue([{ id: 'llama-3.3-70b', contextLength: 128000, maxOutputTokens: 8192, toolSupport: 'supported', supportsImages: false }]);
  renderProviders([]);
  await fillNewProvider();
  await userEvent.click(screen.getByRole('button', { name: 'Fetch models' }));
  await vi.waitFor(() => expect(apiMocks.listProviderModelsDraft).toHaveBeenCalledTimes(1));
  expect(apiMocks.listProviderModelsDraft.mock.calls[0][0]).toMatchObject({ name: 'Acme', baseUrl: 'https://api.example.com/v1' });
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
  expect(await screen.findByRole('status')).toHaveTextContent('found 1 model');
  expect(screen.getByLabelText('Model 1 ID')).toHaveValue('llama-3.3-70b');
  expect(screen.getByLabelText('Model 1 context tokens')).toHaveValue('128000');
});

it('offers local server presets that fill a loopback endpoint with no key required', async () => {
  apiMocks.saveProvider.mockResolvedValue(loopback);
  renderProviders([]);
  await openEditor();
  await userEvent.selectOptions(screen.getByLabelText('Quick setup'), 'ollama');
  expect(screen.getByLabelText('Provider name')).toHaveValue('Ollama');
  expect(screen.getByLabelText('API base URL')).toHaveValue('http://localhost:11434/v1');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledTimes(1));
  expect(apiMocks.saveProvider.mock.calls[0][0]).toMatchObject({ name: 'Ollama', baseUrl: 'http://localhost:11434/v1', apiKey: null });
});

it('keeps Responses explicit when testing and saving the provider draft', async () => {
  apiMocks.testProviderDraft.mockResolvedValue({ verified: true, modelListSupported: true, models: ['fixture-model'], message: 'Connection verified.' });
  apiMocks.saveProvider.mockResolvedValue({ ...verified, apiFormat: 'openai-responses' });
  renderProviders([]); await fillNewProvider();
  await userEvent.selectOptions(screen.getByLabelText('API format'), 'openai-responses');
  expect(screen.getByText(/Native Responses API/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Test connection' }));
  await vi.waitFor(() => expect(apiMocks.testProviderDraft).toHaveBeenCalledWith(expect.objectContaining({ apiFormat: 'openai-responses' })));
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledWith(expect.objectContaining({ apiFormat: 'openai-responses', baseUrl: 'https://api.example.com/v1', apiKey: 'sk-test' })));
});

it('saves explicit inference controls without turning unknown support into supported', async () => {
  apiMocks.saveProvider.mockResolvedValue(verified); renderProviders([]); await fillNewProvider();
  await userEvent.click(screen.getByRole('button', { name: 'Add model ID' }));
  await userEvent.type(screen.getByLabelText('Model 1 ID'), 'custom-model');
  await userEvent.type(screen.getByLabelText('Model 1 context tokens'), '8192');
  await userEvent.click(screen.getByText('Inference controls'));
  await userEvent.click(screen.getByRole('button', { name: 'Configure model 1 inference controls' }));
  await userEvent.selectOptions(screen.getByLabelText('Model 1 structured output support'), 'supported');
  await userEvent.selectOptions(screen.getByLabelText('Model 1 temperature support'), 'unsupported');
  await userEvent.selectOptions(screen.getByLabelText('Model 1 output-token field'), 'max-completion-tokens');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledWith(expect.objectContaining({models: [expect.objectContaining({toolSupport: 'unknown', controls: expect.objectContaining({source: 'configured', structuredOutput: 'supported', temperature: 'unsupported', reasoningEffort: 'unknown', tokenLimitField: 'max-completion-tokens'})})]})));
});

const reportedControls: ModelControls = { source: 'provider-reported', endpointFingerprint: 'a'.repeat(64), temperature: 'supported', topP: 'unknown', reasoningEffort: 'unknown', toolChoice: 'supported', parallelToolCalls: 'unknown', structuredOutput: 'supported', tokenLimitField: 'protocol-default' };
it('preserves reported controls on a name edit but clears them when the endpoint changes', async () => {
  apiMocks.saveProvider.mockResolvedValue(verified);
  renderProviders([{ ...verified, models: [{ ...verified.models[0], controls: reportedControls }] }]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.click(screen.getByText('Inference controls'));
  expect(screen.getByLabelText('Model 1 structured output support')).toHaveValue('supported');
  await userEvent.type(screen.getByLabelText('Provider name'), ' renamed');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledWith(expect.objectContaining({ models: [expect.objectContaining({ controls: reportedControls })] })));
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.type(screen.getByLabelText('API base URL'), '/new');
  expect(screen.queryByLabelText('Model 1 structured output support')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Configure model 1 inference controls' })).toBeInTheDocument();
});

it('keeps an incompatible token field visible and explains how to correct it', async () => {
  renderProviders([{ ...verified, models: [{ ...verified.models[0], controls: { ...reportedControls, source: 'configured', tokenLimitField: 'max-completion-tokens' } }] }]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.selectOptions(screen.getByLabelText('API format'), 'openai-responses');
  await userEvent.type(screen.getByLabelText('API key'), 'new-key');
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  expect(await screen.findByRole('alert')).toHaveTextContent(/output-token field does not match/);
  expect(apiMocks.saveProvider).not.toHaveBeenCalled();
});

it('clears reported optional controls when changing a model ID', async () => {
  renderProviders([{ ...verified, models: [{ ...verified.models[0], controls: reportedControls }] }]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.click(screen.getByText('Inference controls'));
  await userEvent.type(screen.getByLabelText('Model 1 ID'), '-another');
  expect(screen.queryByLabelText('Model 1 structured output support')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Configure model 1 inference controls' })).toBeInTheDocument();
});

it('keeps configured controls, capacities and tool choices when refreshing model metadata', async () => {
  const controls: ModelControls = { ...reportedControls, source: 'configured', structuredOutput: 'unsupported' };
  apiMocks.listProviderModelsDraft.mockResolvedValue([{ ...verified.models[0], contextLength: 128000, maxOutputTokens: 16000, toolSupport: 'supported', supportsImages: true, controls: reportedControls }]);
  apiMocks.saveProvider.mockResolvedValue(verified);
  renderProviders([{ ...verified, models: [{ ...verified.models[0], toolSupport: 'unsupported', controls }] }]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.click(screen.getByRole('button', { name: 'Fetch models' }));
  await vi.waitFor(() => expect(apiMocks.listProviderModelsDraft).toHaveBeenCalledTimes(1));
  await userEvent.click(screen.getByRole('button', { name: 'Save provider' }));
  await vi.waitFor(() => expect(apiMocks.saveProvider).toHaveBeenCalledWith(expect.objectContaining({ models: [expect.objectContaining({ contextLength: 8192, maxOutputTokens: 1024, toolSupport: 'unsupported', supportsImages: false, controls })] })));
});

it('runs a tool-calling probe from the model row and shows the result', async () => {
  apiMocks.probeModelTools.mockResolvedValue({ verified: true, supported: true, detail: 'Model emitted report_probe(value=ok) as a tool call.', probedAt: '1' });
  renderProviders([verified]);
  await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
  await userEvent.click(await screen.findByRole('button', { name: 'Test model 1 tool calling' }));
  expect(apiMocks.probeModelTools).toHaveBeenCalledWith('prov-1', 'acme-1');
  expect(await screen.findByText(/Tool calls verified/)).toBeVisible();
});
