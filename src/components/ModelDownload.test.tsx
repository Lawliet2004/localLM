import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ModelDownload } from './ModelDownload';
const fixtures = vi.hoisted(() => ({ info: vi.fn(), install: vi.fn(), status: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  modelDownloadInfo: fixtures.info, installModel: fixtures.install,
  modelInstallStatus: fixtures.status,
} }));
const info = { filename: 'model.gguf', bytes: 100, requiredBytes: 200, availableBytes: 300, destinationExists: false, destination: 'C:/models/model.gguf', sha256: 'abc' };
it('requests the exact catalog model and never selects another model installer result', async () => {
  const filename = 'MiniCPM5-2B.Q6_K.gguf';
  fixtures.info.mockResolvedValue({...info, filename, destination:`C:/models/${filename}`});
  fixtures.status.mockResolvedValue({busy:false, phase:'ready', path:'C:/models/other.gguf', received:100, total:100});
  render(<ModelDownload busy={false} onSelect={vi.fn()} filename={filename} />);
  expect(await screen.findByText('MiniCPM5-2B · Q6_K')).toBeVisible();
  expect(fixtures.info).toHaveBeenCalledWith(filename);
  expect(screen.queryByRole('button', {name:'Use verified model'})).not.toBeInTheDocument();
});
it('presents cancellation as a normal outcome and allows retry', async () => {
  fixtures.info.mockResolvedValue(info);
  fixtures.status.mockResolvedValue({ busy: false, phase: 'cancelled', received: 10, total: 100, path: null, error: 'Download cancelled.' });
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('status')).toHaveTextContent('Installation cancelled');
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Download model' })).toBeEnabled();
});
it('requires an explicit selection after successful verification', async () => {
  fixtures.info.mockResolvedValue({ ...info, destinationExists: true });
  fixtures.status.mockResolvedValue({ busy: false, phase: '', received: 0, total: 0, path: null, error: null });
  fixtures.install.mockImplementation(async () => { const result = { busy: false, phase: 'ready', received: 100, total: 100, path: info.destination, error: null }; fixtures.status.mockResolvedValue(result); return result; });
  const select = vi.fn();
  render(<ModelDownload busy={false} onSelect={select} />);
  await userEvent.click(await screen.findByRole('button', { name: 'Verify managed model' }));
  const use = await screen.findByRole('button', { name: 'Use verified model' });
  expect(select).not.toHaveBeenCalled();
  await userEvent.click(use);
  expect(select).toHaveBeenCalledWith(info.destination);
});
it('disables a new download when destination space is insufficient', async () => {
  fixtures.status.mockResolvedValue({ busy: false, phase: '', received: 0, total: 0, path: null, error: null });
  fixtures.info.mockResolvedValue({ ...info, availableBytes: 1 });
  render(<ModelDownload busy={false} onSelect={vi.fn()} />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Not enough disk space');
  expect(screen.getByRole('button', { name: 'Download model' })).toBeDisabled();
});
