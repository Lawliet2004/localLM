import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { DownloadActivity } from './DownloadActivity';
const mock = vi.hoisted(() => ({ modelInstallStatus: vi.fn(), cancelModelInstall: vi.fn() }));
vi.mock('../lib/api', () => ({ api: mock, nativeAvailable: true, errorMessage: String }));
beforeEach(() => { vi.clearAllMocks(); });
it('shows progress independently of the models page and pauses the transfer', async () => {
  mock.modelInstallStatus.mockResolvedValue({ busy: true, phase: 'downloading', received: 2 ** 30, total: 4 * 2 ** 30, path: 'C:/models/gemma.gguf', error: null });
  mock.cancelModelInstall.mockResolvedValue(undefined);
  render(<DownloadActivity />);
  expect(await screen.findByText('Downloading model · 25.0%')).toBeVisible();
  expect(screen.getByText('gemma.gguf')).toBeVisible();
  expect(screen.getByRole('progressbar')).toHaveAttribute('value', String(2 ** 30));
  expect(screen.getByText(/You can keep chatting/)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Pause download' }));
  await waitFor(() => expect(mock.cancelModelInstall).toHaveBeenCalledOnce());
});
it('keeps failures visible after leaving the download page', async () => {
  mock.modelInstallStatus.mockResolvedValue({ busy: false, phase: 'failed', received: 100, total: 1000, path: null, error: 'Download stalled for 60 seconds.' });
  render(<DownloadActivity />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Saved partial files can be resumed');
  fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
  expect(screen.queryByLabelText('Model download')).not.toBeInTheDocument();
});
