import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Models } from './Models';
import { defaultRuntimeConfig } from '../lib/types';

vi.mock('./HardwareStatus', () => ({ HardwareStatus: () => null }));
vi.mock('./ModelDownload', () => ({ ModelDownload: () => null }));
vi.mock('./RuntimeDownload', () => ({ RuntimeDownload: () => null }));

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
