import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ModelDownload } from './ModelDownload';
import type { ModelInstallStatus } from '../lib/api';
const fixtures = vi.hoisted(() => ({ info: vi.fn(), install: vi.fn(), cancel: vi.fn(), downloadJobs: vi.fn(async (): Promise<ModelInstallStatus[]> => []) }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  modelDownloadInfo: fixtures.info, installModel: fixtures.install,
  cancelModelInstall: fixtures.cancel, downloadJobs: fixtures.downloadJobs,
} }));
const job = (over: Partial<ModelInstallStatus>): ModelInstallStatus => ({
  busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null,
  jobId: 0, key: 'model:catalog:MiniCPM5-2B.Q6_K.gguf', label: 'model.gguf', asset: null,
  assetIndex: 0, assetCount: 0, assetReceived: 0, assetTotal: 0, saved: 0,
  attempt: 0, notice: null, updatedAt: 0, ...over,
});
const info = { filename: 'model.gguf', bytes: 100, requiredBytes: 200, availableBytes: 300, savedBytes: 0, remainingBytes: 100, destinationExists: false, destination: 'C:/models/model.gguf', sha256: 'abc' };
it('requests the exact catalog model and never selects another model installer result', async () => {
  const filename = 'MiniCPM5-2B.Q6_K.gguf';
  fixtures.info.mockResolvedValue({...info, filename, destination:`C:/models/${filename}`});
  fixtures.downloadJobs.mockResolvedValue([job({ phase:'ready', path:'C:/models/other.gguf', received:100, total:100 })]);
  render(<ModelDownload busy={false} onSelect={vi.fn()} filename={filename} />);
  expect(await screen.findByText('MiniCPM5-2B · Q6_K')).toBeVisible();
  expect(fixtures.info).toHaveBeenCalledWith(filename);
  expect(screen.queryByRole('button', {name:'Use verified model'})).not.toBeInTheDocument();
});
it('presents a paused download as resumable without an alert', async () => {
  fixtures.info.mockResolvedValue(info);
  fixtures.downloadJobs.mockResolvedValue([job({ phase: 'cancelled', received: 10, total: 100, error: 'Download cancelled.' })]);
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByText(/Paused\. Saved bytes are kept/)).toBeVisible();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Resume download' })).toBeEnabled();
});
it('requires an explicit selection after successful verification', async () => {
  fixtures.info.mockResolvedValue({ ...info, destinationExists: true });
  const ready = job({ phase: 'ready', received: 100, total: 100, path: info.destination });
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  fixtures.install.mockImplementation(async () => { fixtures.downloadJobs.mockResolvedValue([ready]); return ready; });
  const select = vi.fn();
  render(<ModelDownload busy={false} onSelect={select} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Verify managed model' }));
  const use = await screen.findByRole('button', { name: 'Use verified model' });
  expect(select).not.toHaveBeenCalled();
  await userEvent.click(use);
  expect(select).toHaveBeenCalledWith(info.destination);
});
it('disables a new download when destination space is insufficient', async () => {
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  fixtures.info.mockResolvedValue({ ...info, availableBytes: 1 });
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Not enough disk space');
  expect(screen.getByRole('button', { name: 'Download model' })).toBeDisabled();
});
it('enables Resume when only the remaining bytes plus reserve fit', async () => {
  fixtures.info.mockResolvedValue({ ...info, bytes: 2 ** 30, savedBytes: 900 * 2 ** 20, remainingBytes: 124 * 2 ** 20, requiredBytes: 380 * 2 ** 20, availableBytes: 400 * 2 ** 20 });
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  const resume = await screen.findByRole('button', { name: 'Resume download' });
  expect(resume).toBeEnabled();
});
it('keeps job progress and Pause visible when the info request fails', async () => {
  fixtures.info.mockRejectedValue(new Error('info unavailable'));
  fixtures.downloadJobs.mockResolvedValue([job({ busy: true, phase: 'downloading', jobId: 8, received: 50, total: 100 })]);
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByText('Downloading')).toBeVisible();
  expect(screen.getByRole('alert')).toHaveTextContent('info unavailable');
  const pause = screen.getByRole('button', { name: 'Pause download' });
  expect(pause).toBeEnabled();
  fireEvent.click(pause);
  await waitFor(() => expect(fixtures.cancel).toHaveBeenCalledWith(8));
});
