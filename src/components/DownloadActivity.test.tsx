import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { DownloadActivity } from './DownloadActivity';
import type { ModelInstallStatus } from '../lib/api';

const mock = vi.hoisted(() => ({
  downloadJobs: vi.fn(async (): Promise<ModelInstallStatus[]> => []),
  cancelModelInstall: vi.fn(), cancelRuntimeInstall: vi.fn(),
  installModel: vi.fn(), installRuntime: vi.fn(),
}));
vi.mock('../lib/api', () => ({ api: mock, nativeAvailable: true, errorMessage: String }));

const job = (over: Partial<ModelInstallStatus>): ModelInstallStatus => ({
  busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null,
  jobId: 0, key: '', label: '', asset: null, assetIndex: 0, assetCount: 0,
  assetReceived: 0, assetTotal: 0, saved: 0, attempt: 0, notice: null, updatedAt: 0,
  ...over,
});

beforeEach(() => { vi.clearAllMocks(); mock.downloadJobs.mockResolvedValue([]); });

it('shows progress for a running job and pauses it', async () => {
  mock.downloadJobs.mockResolvedValue([job({
    busy: true, phase: 'downloading', received: 2 ** 30, total: 4 * 2 ** 30,
    key: 'model:catalog:gemma.gguf', label: 'gemma.gguf', jobId: 2,
  })]);
  mock.cancelModelInstall.mockResolvedValue(undefined);
  render(<DownloadActivity />);
  expect(await screen.findByText('Downloading')).toBeVisible();
  expect(screen.getByRole('region', { name: 'gemma.gguf' })).toBeVisible();
  expect(screen.getByRole('progressbar')).toHaveAttribute('value', String(2 ** 30));
  expect(screen.getByText(/You can keep chatting/)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Pause' }));
  await waitFor(() => expect(mock.cancelModelInstall).toHaveBeenCalledWith(2));
});

it('shows two concurrent jobs and each Pause targets its own API and jobId', async () => {
  mock.downloadJobs.mockResolvedValue([
    job({ busy: true, phase: 'downloading', key: 'runtime:standard', label: 'llama.cpp CUDA runtime', jobId: 7, asset: 'cudart.zip', assetIndex: 2, assetCount: 2, received: 150, total: 200 }),
    job({ busy: true, phase: 'downloading', key: 'model:hf:owner/repo@abc:file.gguf', label: 'file.gguf', jobId: 9, received: 10, total: 100 }),
  ]);
  render(<DownloadActivity />);
  expect(await screen.findByText('Downloading archive 2 of 2')).toBeVisible();
  const runtime = screen.getByRole('region', { name: 'llama.cpp CUDA runtime' });
  const model = screen.getByRole('region', { name: 'file.gguf' });
  fireEvent.click(within(runtime).getByRole('button', { name: 'Pause' }));
  await waitFor(() => expect(mock.cancelRuntimeInstall).toHaveBeenCalledWith(7));
  expect(mock.cancelModelInstall).not.toHaveBeenCalled();
  fireEvent.click(within(model).getByRole('button', { name: 'Pause' }));
  await waitFor(() => expect(mock.cancelModelInstall).toHaveBeenCalledWith(9));
  expect(mock.cancelRuntimeInstall).toHaveBeenCalledTimes(1);
});

it('keeps a terminal error visible after unmount and remount, and dismiss persists', async () => {
  const failed = job({ phase: 'failed', key: 'model:catalog:a.gguf', label: 'a.gguf', jobId: 4, asset: 'a.gguf', error: 'Download stalled for 60 seconds.', received: 100, total: 1000 });
  mock.downloadJobs.mockResolvedValue([failed]);
  const first = render(<DownloadActivity />);
  expect(await screen.findByText('Download stalled for 60 seconds.')).toBeVisible();
  first.unmount();
  render(<DownloadActivity />);
  const row = screen.getByRole('region', { name: 'a.gguf' });
  expect(within(row).getByText('a.gguf', { selector: 'small' })).toBeVisible();
  expect(within(row).getByRole('alert')).toHaveTextContent('Download stalled');
  fireEvent.click(within(row).getByRole('button', { name: 'Dismiss' }));
  expect(screen.queryByRole('region')).not.toBeInTheDocument();
  // Dismissal is module-level, so it survives navigation too.
  render(<DownloadActivity />);
  await waitFor(() => expect(mock.downloadJobs).toHaveBeenCalled());
  expect(screen.queryByRole('region')).not.toBeInTheDocument();
});

it('offers Resume on a paused runtime job and calls its install API', async () => {
  mock.downloadJobs.mockResolvedValue([job({
    phase: 'cancelled', key: 'runtime:standard', label: 'llama.cpp CUDA runtime',
    jobId: 11, received: 100, total: 1000, saved: 100, error: 'Runtime installation cancelled.',
  })]);
  mock.installRuntime.mockResolvedValue(job({ phase: 'ready', key: 'runtime:standard', jobId: 12 }));
  render(<DownloadActivity />);
  expect(await screen.findByText('Paused')).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Resume' }));
  await waitFor(() => expect(mock.installRuntime).toHaveBeenCalledOnce());
});

it('marks an HF job as resumable only from My Models', async () => {
  mock.downloadJobs.mockResolvedValue([job({
    phase: 'failed', key: 'model:hf:owner/repo@abc:file.gguf', label: 'file.gguf',
    jobId: 6, error: 'access denied',
  })]);
  render(<DownloadActivity />);
  expect(await screen.findByText(/Resume in My Models/)).toBeVisible();
  expect(screen.queryByRole('button', { name: 'Resume' })).not.toBeInTheDocument();
});
