import { render, screen, waitFor } from '@testing-library/react';
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
  gpuStatus: 'ok', sampledAt: Date.now(),
  gpus: [{ name: 'RTX', uuid: 'gpu', memoryUsedMib: 1024, memoryTotalMib: 24 * 1024, utilizationPercent: 0, driverVersion: '1' }],
};

describe('runtime settings', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.readModelMetadata.mockImplementation(async (path: string) => path.includes('mmproj') ? { ...llama, fileBytes: 2 ** 30 } : llama);
    api.hardwareStatus.mockResolvedValue(hardware);
  });
  it('saves the explicit MTP-off fallback for Bonsai without changing other model settings', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={{ ...defaultRuntimeConfig, contextLength: 8192 }} onSave={onSave} busy={false}
      modelPath="/models/Ternary-Bonsai-2-9B-PQ2_0-MTP-Q8_0.gguf" />);
    await userEvent.selectOptions(screen.getByLabelText('MTP'), 'off');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ mtpMode: 'off', contextLength: 8192 }));
  });

  it('can disable vision without clearing its configured projector file', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={{ ...defaultRuntimeConfig, contextLength: 8192 }} onSave={onSave} busy={false}
      modelPath="model.gguf" projectorPath="mmproj-model-F16.gguf" />);
    await screen.findByText('1.00 GiB');
    await userEvent.click(screen.getByLabelText('Load vision projector'));
    expect(screen.getByLabelText('Projector placement')).toBeDisabled();
    expect(screen.getByText('mmproj-model-F16.gguf')).toBeVisible();
    expect(screen.getByText('Off · text only')).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ visionEnabled: false }));
  });

  it('can put the projector on CPU independently of language-model GPU layers', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={{ ...defaultRuntimeConfig, contextLength: 8192 }} onSave={onSave} busy={false}
      modelPath="model.gguf" projectorPath="mmproj-model-F16.gguf" />);
    await screen.findByText('1.00 GiB');
    await userEvent.selectOptions(screen.getByLabelText('Projector placement'), 'cpu');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ gpuLayers: -1, visionEnabled: true, offloadProjector: false }));
  });

  it('keeps GPU projector memory visible when the language model is on CPU', async () => {
    render(<RuntimeForm initial={{ ...defaultRuntimeConfig, gpuLayers: 0, contextLength: 8192 }} onSave={vi.fn()} busy={false}
      modelPath="model.gguf" projectorPath="mmproj-model-F16.gguf" />);
    const meter = await screen.findByRole('progressbar', { name: 'Estimated GPU memory' });
    await waitFor(() => expect(meter).toHaveAttribute('value', String(2 ** 30)));
    await userEvent.click(screen.getByLabelText('Load vision projector'));
    expect(meter).toHaveAttribute('value', '0');
  });

  it('distinguishes the loaded projector from pending placement settings', async () => {
    render(<RuntimeForm initial={{ ...defaultRuntimeConfig, contextLength: 8192 }} onSave={vi.fn()} busy={false}
      modelPath="model.gguf" projectorPath="mmproj-model-F16.gguf"
      runtime={{ phase: 'ready', message: 'Model ready', modelPath: 'model.gguf', projector: { path: 'mmproj-model-F16.gguf', offload: false, fileBytes: 2 ** 30 } }} />);
    expect(screen.getByText('On · GPU preferred')).toBeVisible();
    expect(screen.getByText('On · CPU only')).toBeVisible();
  });
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
    expect(screen.getByText('Suggested starting settings')).toBeVisible();
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
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ contextLength: 8192, gpuLayers: -1 }));
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
    expect(await screen.findByText(/Memory fit is unknown/)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Use recommended settings' })).toBeDisabled();
    expect(screen.getAllByText('Unknown').length).toBeGreaterThan(0);
    expect(screen.getByText(/cache layout incomplete/)).toBeVisible();
  });
});
