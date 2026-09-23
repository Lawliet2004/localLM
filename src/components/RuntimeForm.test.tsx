import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { RuntimeForm } from './RuntimeForm';
import { defaultRuntimeConfig, type HardwareStatus, type ModelMetadata } from '../lib/types';

const api = vi.hoisted(() => ({
  readModelMetadata: vi.fn(),
  hardwareStatus: vi.fn(),
}));
vi.mock('../lib/api', () => ({ api, nativeAvailable: true, errorMessage: String }));

const llama: ModelMetadata = {
  architecture: 'llama', contextLength: 32768, blockCount: 32, embeddingLength: 4096,
  headCount: 32, headCountKv: 8, keyLength: null, valueLength: null, fileBytes: 4 * 2 ** 30,
};
const hardware: HardwareStatus = {
  logicalCpus: 12, memoryTotalBytes: 16 * 2 ** 30, memoryAvailableBytes: 8 * 2 ** 30,
  gpuStatus: 'ok', sampledAt: 0,
  gpus: [{ name: 'RTX', uuid: 'gpu', memoryUsedMib: 1024, memoryTotalMib: 24 * 1024, utilizationPercent: 0, driverVersion: '1' }],
};

describe('runtime settings', () => {
  beforeEach(() => vi.clearAllMocks());
  it('says saving will reload when a model is already loaded', () => {
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={vi.fn()} busy={false}
      runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', loadedConfig: defaultRuntimeConfig }} />);
    expect(screen.getByText(/Saving reloads the model/)).toBeVisible();
  });

  it('defaults to automatic GPU fill and can save CPU-only placement', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    expect(screen.getByLabelText('Compute device')).toHaveValue('auto');
    await userEvent.selectOptions(screen.getByLabelText('Compute device'), 'cpu');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ gpuLayers: 0, offloadKvCache: false, contextLength: 131072 }));
  });

  it('prevents saving quantized value cache without Flash Attention', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    await userEvent.click(screen.getByText('Advanced memory and batch settings'));
    await userEvent.click(screen.getByLabelText('Flash Attention'));
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Flash Attention');
  });

  it('does not let users submit a micro batch larger than the batch', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    await userEvent.click(screen.getByText('Advanced memory and batch settings'));
    const input = screen.getByLabelText('Micro batch size');
    await userEvent.clear(input);
    await userEvent.type(input, '1024');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Micro batch');
  });

  it('shows model layer, context, memory, and recommended limits', async () => {
    api.readModelMetadata.mockResolvedValue(llama);
    api.hardwareStatus.mockResolvedValue(hardware);
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={vi.fn()} busy={false} modelPath="C:/models/model.gguf" />);
    expect(await screen.findByText('33 layers in this model')).toBeVisible();
    expect(screen.getAllByText('32,768 tokens').length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText('All layers')).toBeVisible();
    expect(screen.getByText('Recommended for this model')).toBeVisible();
    expect(screen.getByRole('progressbar', { name: 'Estimated GPU memory' })).toBeVisible();
  });

  it('applies recommended settings for the selected model', async () => {
    const onSave = vi.fn();
    api.readModelMetadata.mockResolvedValue(llama);
    api.hardwareStatus.mockResolvedValue(hardware);
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} modelPath="C:/models/model.gguf" />);
    await screen.findByRole('button', { name: 'Use recommended settings' });
    await userEvent.click(screen.getByRole('button', { name: 'Use recommended settings' }));
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ contextLength: 32768, gpuLayers: -1 }));
  });

  it('rejects a context larger than the model maximum', async () => {
    const onSave = vi.fn();
    api.readModelMetadata.mockResolvedValue({ ...llama, contextLength: 2048 });
    api.hardwareStatus.mockResolvedValue(hardware);
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} modelPath="C:/models/model.gguf" />);
    expect(await screen.findByRole('alert')).toHaveTextContent('exceeds this model');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it('does not invent cache cost for hybrid architectures', async () => {
    api.readModelMetadata.mockResolvedValue({ ...llama, architecture: 'hybridnet' });
    api.hardwareStatus.mockResolvedValue(hardware);
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={vi.fn()} busy={false} modelPath="C:/models/hybrid.gguf" />);
    expect(await screen.findByText('CPU recommended')).toBeVisible();
    expect(screen.getByText('Unknown')).toBeVisible();
    expect(screen.getByText(/cache layout incomplete/)).toBeVisible();
  });
});
