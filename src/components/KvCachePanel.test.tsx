import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { KvCachePanel } from './KvCachePanel';

const usage = (enabled: boolean, files: number) => ({ settings: { enabled, budgetMb: 4096 }, files, bytes: files * 1024 * 1024 * 300, runtimeSupported: true });
const apiMocks = vi.hoisted(() => ({ kvCacheUsage: vi.fn(), saveKvCacheSettings: vi.fn(), clearKvCache: vi.fn() }));
vi.mock('../lib/api', () => ({ api: apiMocks, nativeAvailable: true, errorMessage: String }));

it('shows usage, deletes saved files and turns saving off', async () => {
  apiMocks.kvCacheUsage.mockResolvedValue(usage(true, 2));
  apiMocks.clearKvCache.mockResolvedValue(usage(true, 0));
  apiMocks.saveKvCacheSettings.mockResolvedValue(usage(false, 0));
  render(<KvCachePanel />);
  expect(await screen.findByText('2 saved · 600 MB')).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Delete saved cache' }));
  expect(await screen.findByText('0 saved · 0 MB')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Delete saved cache' })).toBeDisabled();
  await userEvent.click(screen.getByLabelText('Save conversation cache'));
  expect(apiMocks.saveKvCacheSettings).toHaveBeenCalledWith({ enabled: false, budgetMb: 4096 });
  expect(await screen.findByLabelText('Save conversation cache')).not.toBeChecked();
});
