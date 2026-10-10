import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { JobProgress } from './JobProgress';
import type { ModelInstallStatus } from '../lib/api';

const job = (over: Partial<ModelInstallStatus>): ModelInstallStatus => ({
  busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null,
  jobId: 1, key: 'runtime:standard', label: 'llama.cpp CUDA runtime', asset: null,
  assetIndex: 0, assetCount: 0, assetReceived: 0, assetTotal: 0, saved: 0,
  attempt: 0, notice: null, updatedAt: 0, ...over,
});

it('shows the current archive and aggregate bytes for a multi-archive runtime download', () => {
  render(<JobProgress job={job({ busy: true, phase: 'downloading', asset: 'b.zip', assetIndex: 2, assetCount: 2, received: 0.5 * 2 ** 30, total: 2 ** 30, assetReceived: 512, assetTotal: 1024 })} />);
  expect(screen.getByText('Downloading archive 2 of 2')).toBeVisible();
  expect(screen.getByText(/0\.50 GiB \/ 1\.00 GiB/)).toBeInTheDocument();
  expect(screen.getByText('b.zip')).toBeVisible();
});

it('renders an indeterminate bar and no rate while extracting', () => {
  render(<JobProgress job={job({ busy: true, phase: 'extracting', asset: 'a.zip', assetIndex: 1, assetCount: 2, received: 2048, total: 2048 })} rate={500} />);
  expect(screen.getByText('Extracting a.zip (1 of 2)')).toBeVisible();
  expect(screen.getByRole('progressbar')).not.toHaveAttribute('value');
  expect(screen.queryByText(/MiB\/s/)).not.toBeInTheDocument();
});

it('labels verification separately from downloading', () => {
  render(<JobProgress job={job({ busy: true, phase: 'verifying', asset: 'a.zip', assetIndex: 1, assetCount: 2, received: 1024, total: 2048, assetReceived: 400, assetTotal: 1024 })} />);
  expect(screen.getByText('Verifying a.zip (1 of 2)')).toBeVisible();
  expect(screen.queryByText(/Downloading/)).not.toBeInTheDocument();
  expect(screen.getByRole('progressbar', { name: 'Verification progress' })).toHaveAttribute('value', '400');
});

it('reports bytes already saved at job start', () => {
  render(<JobProgress job={job({ busy: true, phase: 'resuming', key: 'model:catalog:m.gguf', label: 'm.gguf', received: 600, total: 1000, saved: 500 })} />);
  expect(screen.getByText(/0\.00 GiB were already saved/)).toBeInTheDocument();
});

it('shows a paused error as status rather than an alert', () => {
  render(<JobProgress job={job({ phase: 'cancelled', error: 'Download cancelled.', received: 100, total: 1000 })} />);
  expect(screen.getByText('Paused')).toBeVisible();
  expect(screen.getByText(/Paused\. Saved bytes are kept; Resume continues from them\./)).toBeVisible();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
