import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Models } from './Models';
import { defaultRuntimeConfig } from '../lib/types';

const apiMocks = vi.hoisted(() => ({ modelDownloadInfo: vi.fn() }));
vi.mock('../lib/api', () => ({ api: apiMocks, nativeAvailable: false, errorMessage: String }));

vi.mock('./HardwareStatus', () => ({ HardwareStatus: () => null }));
vi.mock('./ModelDownload', () => ({ ModelDownload: () => null }));
vi.mock('./RuntimeDownload', () => ({ RuntimeDownload: () => null }));

it('saves a vision projector path with the model files', async () => {
  const onSavePreferences = vi.fn().mockResolvedValue(undefined);
  render(<Models config={defaultRuntimeConfig}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'stopped', message: 'No model loaded', modelPath: null }}
    providers={[]} selection={{ providerId: null, modelId: '' }} onSaveSelection={vi.fn()} onProvidersChanged={vi.fn()}
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
    providers={[]} selection={{ providerId: null, modelId: '' }} onSaveSelection={vi.fn()} onProvidersChanged={vi.fn()}
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
    providers={[]} selection={{ providerId: null, modelId: '' }} onSaveSelection={vi.fn()} onProvidersChanged={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  await userEvent.click(screen.getByRole('tab', { name: 'Generation' }));
  expect(screen.getByRole('alert')).toHaveTextContent('entire');
});

it('lets users apply saved context to an already loaded model', async () => {
  const onLoad = vi.fn();
  render(<Models config={{ ...defaultRuntimeConfig, contextLength: 131072 }}
    preferences={{ runtimePath: 'server.exe', modelPath: 'model.gguf', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', loadedConfig: { ...defaultRuntimeConfig, contextLength: 8192 } }}
    providers={[]} selection={{ providerId: null, modelId: '' }} onSaveSelection={vi.fn()} onProvidersChanged={vi.fn()}
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
    providers={[]} selection={{ providerId: null, modelId: '' }} onSaveSelection={vi.fn()} onProvidersChanged={vi.fn()}
    busy={false} onSaveConfig={vi.fn()} onSavePreferences={vi.fn()} onLoad={vi.fn()} onUnload={vi.fn()} />);
  expect(screen.queryByRole('button', { name: /Switch to/ })).not.toBeInTheDocument();
  expect(screen.queryByText(/A different model file is saved/)).not.toBeInTheDocument();
});

it('consumes a chat model request once and saves the requested file', async () => {
  apiMocks.modelDownloadInfo.mockResolvedValueOnce({ destinationExists: true, destination: 'C:\\models\\Ternary-Bonsai-8B-Q2_0.gguf' });
  const onSavePreferences = vi.fn().mockResolvedValue(undefined);
  const onRequestedModelHandled = vi.fn();
  render(<Models requestedModel="Ternary-Bonsai-8B-Q2_0.gguf" onRequestedModelHandled={onRequestedModelHandled}
    config={defaultRuntimeConfig}
    preferences={{ runtimePath: 'C:\\runtime\\llama-server.exe', modelPath: 'C:\\models\\MiniCPM5-2B.Q6_K.gguf', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' }}
    runtime={{ phase: 'stopped', message: 'No model loaded', modelPath: null }} providers={[]}
    selection={{ providerId: null, modelId: '' }} busy={false} onSaveConfig={vi.fn()} onSavePreferences={onSavePreferences}
    onLoad={vi.fn()} onUnload={vi.fn()} />);
  await vi.waitFor(() => expect(onSavePreferences).toHaveBeenCalled());
  expect(onSavePreferences.mock.calls[0][0].modelPath).toContain('Ternary-Bonsai-8B-Q2_0.gguf');
  expect(onRequestedModelHandled).toHaveBeenCalledOnce();
});
