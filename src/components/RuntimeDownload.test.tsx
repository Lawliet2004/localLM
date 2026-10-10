import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RuntimeDownload } from './RuntimeDownload';
import type { ModelInstallStatus } from '../lib/api';
const fixtures = vi.hoisted(() => ({ info: vi.fn(), install: vi.fn(), downloadJobs: vi.fn(async (): Promise<ModelInstallStatus[]> => []) }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: { listInstalledRuntimes: async () => [], runtimeDownloadInfo: fixtures.info, installRuntime: fixtures.install, downloadJobs: fixtures.downloadJobs } }));
const job = (over: Partial<ModelInstallStatus>): ModelInstallStatus => ({
  busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null,
  jobId: 0, key: 'runtime:standard', label: 'llama.cpp CUDA runtime', asset: null,
  assetIndex: 0, assetCount: 0, assetReceived: 0, assetTotal: 0, saved: 0,
  attempt: 0, notice: null, updatedAt: 0, ...over,
});
const info = { bytes: 1, requiredBytes: 2, availableBytes: 3, savedBytes: 0, reusable: false, destination: 'C:/runtimes' };
it('preserves selection until the user selects a completed installation', async () => {
  fixtures.info.mockResolvedValue(info);
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  const ready = job({ phase: 'ready', path: 'C:/runtimes/new/llama-server.exe' });
  fixtures.install.mockImplementation(async () => { fixtures.downloadJobs.mockResolvedValue([ready]); return ready; });
  const select = vi.fn();
  render(<RuntimeDownload busy={false} onSelect={select} />);
  await screen.findByText(/required incl\. reserve/);
  await userEvent.click(screen.getByRole('button', { name: 'Install CUDA runtime' }));
  const button = await screen.findByRole('button', { name: 'Use installed runtime' });
  expect(select).not.toHaveBeenCalled();
  await userEvent.click(button);
  expect(select).toHaveBeenCalledWith('C:/runtimes/new/llama-server.exe');
});
it('offers Use when a pinned runtime is already installed', async () => {
  fixtures.info.mockResolvedValue({ ...info, reusable: true });
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  render(<RuntimeDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('button', { name: 'Use installed runtime' })).toBeEnabled();
});
it('offers Resume when archives are already saved', async () => {
  fixtures.info.mockResolvedValue({ ...info, bytes: 2 ** 30, savedBytes: 400 * 2 ** 20, requiredBytes: 900 * 2 ** 20, availableBytes: 2 ** 30 });
  fixtures.downloadJobs.mockResolvedValue([job({})]);
  render(<RuntimeDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('button', { name: 'Resume runtime download' })).toBeEnabled();
});
