import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { BonsaiSettings } from './BonsaiSettings';
import { defaultRuntimeConfig } from '../../lib/types';

const mocks = vi.hoisted(() => ({
  bonsaiEnabled: vi.fn(), setBonsaiEnabled: vi.fn(), bonsaiInfo: vi.fn(), activateBonsai: vi.fn(),
}));
vi.mock('../../lib/api', () => ({ api: mocks, nativeAvailable: true, errorMessage: String }));
vi.mock('../../components/ModelDownload', () => ({ ModelDownload: () => null }));
vi.mock('../../components/RuntimeDownload', () => ({ RuntimeDownload: () => null }));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.bonsaiEnabled.mockResolvedValue(false);
  mocks.setBonsaiEnabled.mockResolvedValue(true);
  mocks.bonsaiInfo.mockResolvedValue(null);
});

it('keeps the experimental catalog and acquisition calls absent while off', async () => {
  render(<BonsaiSettings busy={false} modelPath="MiniCPM5-2B.Q6_K.gguf" onActivated={vi.fn()} onPreset={vi.fn()} />);
  await waitFor(() => expect(mocks.bonsaiEnabled).toHaveBeenCalledOnce());
  expect(screen.getByRole('checkbox', { name: 'Experimental models' })).not.toBeChecked();
  expect(screen.queryByText(/Bonsai/)).not.toBeInTheDocument();
  expect(mocks.bonsaiInfo).not.toHaveBeenCalled();
});

it('reveals the model and warning only after explicit opt-in', async () => {
  render(<BonsaiSettings busy={false} modelPath="MiniCPM5-2B.Q6_K.gguf" onActivated={vi.fn()} onPreset={vi.fn()} />);
  await userEvent.click(await screen.findByRole('checkbox', { name: 'Experimental models' }));
  await waitFor(() => expect(mocks.setBonsaiEnabled).toHaveBeenCalledWith(true));
  expect(await screen.findByText('Experimental')).toBeInTheDocument();
  expect(screen.getByText(/Bonsai-27B/)).toBeInTheDocument();
  expect(screen.getByText(/1-bit quality/)).toBeInTheDocument();
  await waitFor(() => expect(mocks.bonsaiInfo).toHaveBeenCalled());
});

it('keeps the feature off when saving opt-in fails', async () => {
  mocks.setBonsaiEnabled.mockRejectedValueOnce(new Error('Settings unavailable'));
  render(<BonsaiSettings busy={false} modelPath="MiniCPM5-2B.Q6_K.gguf" onActivated={vi.fn()} onPreset={vi.fn()} />);
  await userEvent.click(await screen.findByRole('checkbox', { name: 'Experimental models' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Settings unavailable');
  expect(screen.queryByText(/Bonsai-27B/)).not.toBeInTheDocument();
});

it('applies the measured preset through the existing load-settings callback for the selected model', async () => {
  mocks.bonsaiEnabled.mockResolvedValue(true);
  const preset = { ...defaultRuntimeConfig, contextLength: 4096, gpuLayers: 65 };
  mocks.bonsaiInfo.mockResolvedValue({ model: { destinationExists: true }, runtimePath: 'PrismML/llama-server.exe', preset });
  const onPreset = vi.fn();
  render(<BonsaiSettings busy={false} modelPath={'C:\\app\\experimental\\bonsai-27b\\models\\Bonsai-27B-Q1_0.gguf'} onActivated={vi.fn()} onPreset={onPreset} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Apply measured Bonsai preset' }));
  expect(onPreset).toHaveBeenCalledWith(preset);
  expect(mocks.activateBonsai).not.toHaveBeenCalled();
});

it('activates through the native gate when applying a preset to a different model', async () => {
  mocks.bonsaiEnabled.mockResolvedValue(true);
  mocks.bonsaiInfo.mockResolvedValue({ model: { destinationExists: true }, runtimePath: 'PrismML/llama-server.exe', preset: defaultRuntimeConfig });
  mocks.activateBonsai.mockResolvedValue({ phase: 'ready' });
  const activated = vi.fn();
  render(<BonsaiSettings busy={false} modelPath="MiniCPM5-2B.Q6_K.gguf" onActivated={activated} onPreset={vi.fn()} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Apply measured Bonsai preset' }));
  expect(mocks.activateBonsai).toHaveBeenCalledWith(true);
  await waitFor(() => expect(activated).toHaveBeenCalledOnce());
});
