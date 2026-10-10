import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Models } from './Models';
import { defaultRuntimeConfig } from '../lib/types';

const apiMocks = vi.hoisted(() => ({ modelDownloadInfo: vi.fn(), useInstalledModel: vi.fn() }));
vi.mock('../lib/api', () => ({ api: apiMocks, nativeAvailable: false, errorMessage: String }));

vi.mock('./HardwareStatus', () => ({ HardwareStatus: () => null }));
vi.mock('./ModelDownload', () => ({ ModelDownload: () => null }));
vi.mock('./RuntimeDownload', () => ({ RuntimeDownload: () => null }));
vi.mock('./ModelLibrary', () => ({ ModelLibrary: ({ onUse }: { onUse: (model: unknown) => Promise<void> }) => <button onClick={() => void onUse({
  id: 'C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf', path: 'C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf',
  filename: 'BAAI_AREX-Turbo-Q4_K_M.gguf', projectorPath: 'C:/models/mmproj-BAAI_AREX-Turbo-f16.gguf',
})}>Use AREX</button> }));

it('does not restore the previous model path after loading a model with a projector', async () => {
  apiMocks.useInstalledModel.mockResolvedValueOnce({ phase: 'ready', message: 'Model ready', modelPath: 'C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf' });
  const onSavePreferences = vi.fn().mockResolvedValue(undefined);
  const onRefresh = vi.fn().mockResolvedValue(undefined);
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 57500 }}
    preferences={{ runtimePath: 'server.exe', modelPath: 'C:/models/K2-Horizon-4B-Q4_K_M.gguf', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'C:/models/K2-Horizon-4B-Q4_K_M.gguf' }}
    onSaveSelection={vi.fn().mockResolvedValue(undefined)}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences} onRefresh={onRefresh}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('button', { name: 'Use AREX' }));
  await vi.waitFor(() => expect(onRefresh).toHaveBeenCalledOnce());
  expect(apiMocks.useInstalledModel).toHaveBeenCalledWith('C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf');
  expect(onSavePreferences).not.toHaveBeenCalled();
});

it('saves a vision projector path with the model files', async () => {
  const onSavePreferences = vi.fn().mockResolvedValue(undefined);
  render(<Models config={defaultRuntimeConfig}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'stopped', message: 'No model loaded', modelPath: null }}
    onSaveSelection={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences} onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('tab', { name: 'Model files' }));
  await userEvent.type(screen.getByLabelText('Vision projector file'), 'C:\\models\\mmproj.gguf');
  await userEvent.click(screen.getByRole('button', { name: 'Save settings' }));
  await vi.waitFor(() => expect(onSavePreferences).toHaveBeenCalled());
  expect(onSavePreferences.mock.calls[0][0].projectorPath).toBe('C:\\models\\mmproj.gguf');
});

it('reloads a loaded model after saving runtime configuration', async () => {
  const onLoad = vi.fn();
  const onSaveConfig = vi.fn().mockResolvedValue(undefined);
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 8192 }}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', temperature: 1, topP: 0.95, maxTokens: 8192, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', loadedConfig: { ...defaultRuntimeConfig, contextLength: 8192 } }}
    onSaveSelection={vi.fn()}
    busy={false} onSaveConfig={onSaveConfig} onSavePreferences={vi.fn()} onLoad={onLoad} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('tab', { name: 'Runtime' }));
  await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
  await vi.waitFor(() => expect(onSaveConfig).toHaveBeenCalled());
  expect(onLoad).toHaveBeenCalledOnce();
});

it('warns when the response reserve uses the entire context window', async () => {
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 8192 }}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', temperature: 1, topP: 0.95, maxTokens: 8192, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', loadedConfig: { ...defaultRuntimeConfig, contextLength: 8192 } }}
    onSaveSelection={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('tab', { name: 'Generation' }));
  expect(screen.getByRole('alert')).toHaveTextContent('entire');
});

it('lets users apply saved context to an already loaded model', async () => {
  const onLoad = vi.fn();
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 131072 }}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', loadedConfig: { ...defaultRuntimeConfig, contextLength: 8192 } }}
    onSaveSelection={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={onLoad} onUnload={vi.fn()} />);
  expect(screen.getByRole('status')).toHaveTextContent((131072).toLocaleString());
  expect(screen.getByRole('status')).toHaveTextContent((8192).toLocaleString());
  await userEvent.click(screen.getByRole('button', { name: 'Apply saved configuration' }));
  expect(onLoad).toHaveBeenCalledOnce();
});

it('does not show a switch prompt when Windows canonicalization added a device prefix', () => {
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 8192 }}
    preferences={{ runtimePath: 'C:\\runtime\\llama-server.exe', modelPath: 'C:\\models\\model.gguf', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: '\\\\?\\C:\\models\\model.gguf', loadedConfig: { ...defaultRuntimeConfig, contextLength: 8192 } }}
    onSaveSelection={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  expect(screen.queryByRole('button', { name: /Switch to/ })).not.toBeInTheDocument();
  expect(screen.queryByText(/A different model file is saved/)).not.toBeInTheDocument();
});

const prefs = (over: Partial<import('../lib/types').Preferences> = {}) => ({
  runtimePath: 'server.exe', modelPath: 'model.gguf', projectorPath: '',
  temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '', ...over,
});
const stopped = { phase: 'stopped' as const, message: 'No model loaded', modelPath: null };

it('sends only one preferences save while the first is still pending', async () => {
  let resolve: (value: string | undefined) => void = () => {};
  const onSavePreferences = vi.fn(() => new Promise<string | undefined>(done => { resolve = done; }));
  render(<Models section="generation" config={defaultRuntimeConfig} preferences={prefs()}
    runtime={stopped} busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('button', { name: 'Save settings' }));
  const saving = await screen.findByRole('button', { name: 'Saving…' });
  expect(saving).toBeDisabled();
  await userEvent.click(saving);
  expect(onSavePreferences).toHaveBeenCalledTimes(1);
  resolve(undefined);
  expect(await screen.findByRole('button', { name: 'Save settings' })).toBeEnabled();
});

it('shows the runtime status text once when it only restates the title', () => {
  render(<Models section="runtime" config={defaultRuntimeConfig} preferences={prefs()}
    runtime={stopped} busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  expect(screen.getAllByText('No model loaded')).toHaveLength(1);
});

it('moves tab selection and focus with arrow keys', async () => {
  render(<Models config={defaultRuntimeConfig} preferences={prefs()}
    runtime={stopped} busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  const list = screen.getByRole('tablist', { name: 'Model settings' });
  const models = within(list).getByRole('tab', { name: 'Models' });
  const runtime = within(list).getByRole('tab', { name: 'Runtime' });
  expect(models).toHaveAttribute('aria-selected', 'true');
  expect(models).toHaveAttribute('tabIndex', '0');
  expect(runtime).toHaveAttribute('tabIndex', '-1');
  fireEvent.keyDown(list, { key: 'ArrowRight' });
  await waitFor(() => expect(within(list).getByRole('tab', { name: 'Runtime' })).toHaveFocus());
  expect(within(list).getByRole('tab', { name: 'Runtime' })).toHaveAttribute('aria-selected', 'true');
});

it('keeps unsaved draft edits when persisted preferences change underneath', async () => {
  const { rerender } = render(<Models section="generation" config={defaultRuntimeConfig}
    preferences={prefs()} runtime={stopped} busy={false} onSaveConfig={vi.fn()}
    onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  const temperature = screen.getByLabelText('Temperature');
  await userEvent.clear(temperature);
  await userEvent.type(temperature, '1.5');
  rerender(<Models section="generation" config={defaultRuntimeConfig}
    preferences={prefs({ temperature: 0.5, systemPrompt: 'saved elsewhere' })} runtime={stopped}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  expect(screen.getByLabelText('Temperature')).toHaveValue(1.5);
  expect(screen.getByLabelText('System instructions')).toHaveValue('');
});

it('adopts refreshed preferences while the draft is clean', async () => {
  const { rerender } = render(<Models section="generation" config={defaultRuntimeConfig}
    preferences={prefs()} runtime={stopped} busy={false} onSaveConfig={vi.fn()}
    onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  rerender(<Models section="generation" config={defaultRuntimeConfig}
    preferences={prefs({ temperature: 0.5 })} runtime={stopped}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  expect(screen.getByLabelText('Temperature')).toHaveValue(0.5);
});

it('shows the refresh-failure message when the save itself succeeded', async () => {
  const onSavePreferences = vi.fn().mockResolvedValue('Saved, but settings could not be refreshed: offline');
  render(<Models section="generation" config={defaultRuntimeConfig} preferences={prefs()}
    runtime={stopped} busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('button', { name: 'Save settings' }));
  expect(await screen.findByRole('status')).toHaveTextContent('Saved, but settings could not be refreshed: offline');
});

it('consumes a chat model request once and saves the requested file', async () => {
  apiMocks.modelDownloadInfo.mockResolvedValueOnce({ destinationExists: true, destination: 'C:\\models\\MiniCPM5-2B.Q6_K.gguf' });
  const onSavePreferences = vi.fn().mockResolvedValue(undefined);
  const onRequestedModelHandled = vi.fn();
  render(<Models requestedModel="MiniCPM5-2B.Q6_K.gguf" onRequestedModelHandled={onRequestedModelHandled}
    config={defaultRuntimeConfig}
    preferences={{ runtimePath: 'C:\\runtime\\llama-server.exe', modelPath: 'C:\\models\\other.gguf', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'stopped', message: 'No model loaded', modelPath: null }}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  await vi.waitFor(() => expect(onSavePreferences).toHaveBeenCalled());
  expect(onSavePreferences.mock.calls[0][0].modelPath).toContain('MiniCPM5-2B.Q6_K.gguf');
  expect(onRequestedModelHandled).toHaveBeenCalledOnce();
});
