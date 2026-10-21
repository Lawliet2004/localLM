import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RuntimeDownload } from './RuntimeDownload';
const fixtures = vi.hoisted(() => ({ info: vi.fn(), status: vi.fn(), install: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: { listInstalledRuntimes: async () => [], runtimeDownloadInfo: fixtures.info, runtimeInstallStatus: fixtures.status, installRuntime: fixtures.install } }));
it('preserves selection until the user selects a completed installation', async () => {
  fixtures.info.mockResolvedValue({ bytes: 1, requiredBytes: 2, availableBytes: 3, destination: 'C:/runtimes' });
  fixtures.status.mockResolvedValue({ busy: false, phase: '', path: null });
  fixtures.install.mockImplementation(async () => { const result = { busy: false, phase: 'ready', path: 'C:/runtimes/new/llama-server.exe' }; fixtures.status.mockResolvedValue(result); return result; });
  const select = vi.fn();
  render(<RuntimeDownload busy={false} onSelect={select} />);
  await screen.findByText(/required for download/);
  await userEvent.click(screen.getByRole('button', { name: 'Install CUDA runtime' }));
  const button = await screen.findByRole('button', { name: 'Use installed runtime' });
  expect(select).not.toHaveBeenCalled();
  await userEvent.click(button);
  expect(select).toHaveBeenCalledWith('C:/runtimes/new/llama-server.exe');
});

it('shows native runtime guidance on Linux without offering Windows downloads', async () => {
  fixtures.info.mockResolvedValue({ bytes: 0, requiredBytes: 0, availableBytes: 1024, destination: '/runtimes', supported: false, platform: 'linux', message: 'Select a native llama-server executable.' });
  fixtures.status.mockResolvedValue({ busy: false, phase: 'idle', path: null });
  fixtures.install.mockClear();
  render(<RuntimeDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('heading', { name: 'llama.cpp · Linux' })).toBeVisible();
  expect(screen.getByText('Select a native llama-server executable.')).toBeVisible();
  expect(screen.queryByRole('button', { name: 'Install CUDA runtime' })).not.toBeInTheDocument();
  expect(fixtures.install).not.toHaveBeenCalled();
});
