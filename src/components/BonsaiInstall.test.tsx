import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { BonsaiInstall } from './BonsaiInstall';
const mock = vi.hoisted(() => ({ choices: vi.fn(), hardware: vi.fn(), info: vi.fn(), install: vi.fn(), modelStatus: vi.fn(), runtimeStatus: vi.fn(), cancel: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  prismRuntimeChoices: mock.choices, hardwareStatus: mock.hardware, runtimeDownloadInfo: mock.info,
  installBonsai: mock.install, modelInstallStatus: mock.modelStatus, runtimeInstallStatus: mock.runtimeStatus,
  cancelBonsaiInstall: mock.cancel,
} }));
beforeEach(() => {
  vi.clearAllMocks();
  mock.choices.mockResolvedValue([{ id: 'prism-linux-cpu', backend: 'CPU', build: 'pinned' }, { id: 'prism-linux-cuda', backend: 'CUDA 12.4', build: 'pinned' }]);
  mock.hardware.mockResolvedValue({ gpus: [{ name: 'NVIDIA GeForce RTX 2050' }] });
  mock.info.mockResolvedValue({ bytes: 1000, requiredBytes: 2000, availableBytes: 10e9, destination: '/data/runtimes' });
  mock.modelStatus.mockResolvedValue({ busy: false, phase: 'idle', received: 0, total: 0 });
  mock.runtimeStatus.mockResolvedValue({ busy: false, phase: 'idle', received: 0, total: 0 });
  mock.install.mockResolvedValue({ phase: 'ready' });
  mock.cancel.mockResolvedValue(undefined);
});
it('selects CUDA for NVIDIA and installs the runtime/model as one backend operation', async () => {
  const refresh = vi.fn().mockResolvedValue(undefined);
  render(<BonsaiInstall busy={false} onInstalled={refresh} />);
  const button = await screen.findByRole('button', { name: 'Install and use Bonsai' });
  await waitFor(() => expect(button).toBeEnabled());
  expect(screen.getByLabelText('Prism backend')).toHaveValue('prism-linux-cuda');
  fireEvent.click(button);
  await waitFor(() => expect(mock.install).toHaveBeenCalledWith('prism-linux-cuda', 'auto'));
  await waitFor(() => expect(refresh).toHaveBeenCalledWith(true));
  expect(await screen.findByText(/Bonsai is loaded/)).toBeVisible();
});
it('refreshes failed installs without selecting a provider and offers an explicit MTP-off retry', async () => {
  mock.install.mockRejectedValueOnce(new Error('MTP startup failed'));
  const refresh = vi.fn().mockResolvedValue(undefined);
  render(<BonsaiInstall busy={false} onInstalled={refresh} />);
  const button = await screen.findByRole('button', { name: 'Install and use Bonsai' });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  expect(await screen.findByRole('alert')).toHaveTextContent('MTP startup failed');
  await waitFor(() => expect(refresh).toHaveBeenCalledWith(false));
  fireEvent.click(screen.getByRole('button', { name: 'Install and run without MTP' }));
  await waitFor(() => expect(mock.install).toHaveBeenLastCalledWith('prism-linux-cuda', 'off'));
});
it('blocks the combined install when there is insufficient space for both components', async () => {
  mock.info.mockResolvedValue({ bytes: 1, requiredBytes: 2000, availableBytes: 2675410721, destination: '/data/runtimes' });
  render(<BonsaiInstall busy={false} onInstalled={vi.fn()} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Not enough disk space');
  expect(screen.getByRole('button', { name: 'Install and use Bonsai' })).toBeDisabled();
  expect(mock.install).not.toHaveBeenCalled();
});
it('cancels only the owned Bonsai operation', async () => {
  mock.install.mockReturnValue(new Promise(() => {}));
  render(<BonsaiInstall busy={false} onInstalled={vi.fn()} />);
  const button = await screen.findByRole('button', { name: 'Install and use Bonsai' });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  fireEvent.click(await screen.findByRole('button', { name: 'Cancel Bonsai installation' }));
  await waitFor(() => expect(mock.cancel).toHaveBeenCalledTimes(1));
});

it('does not expose Bonsai cancellation for an unrelated installation', async () => {
  mock.modelStatus.mockResolvedValue({ busy: true, phase: 'downloading', received: 1, total: 2 });
  render(<BonsaiInstall busy={false} onInstalled={vi.fn()} />);
  expect(await screen.findByText(/Another model or runtime installation/)).toBeVisible();
  expect(screen.getByRole('button', { name: 'Install and use Bonsai' })).toBeDisabled();
  expect(screen.queryByRole('button', { name: 'Cancel Bonsai installation' })).not.toBeInTheDocument();
});
