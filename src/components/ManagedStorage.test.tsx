import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { ManagedStorage } from './ManagedStorage';

const mock = vi.hoisted(() => ({ info: vi.fn(), move: vi.fn(), open: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mock.open }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: { managedStorageInfo: mock.info, moveManagedStorage: mock.move } }));
beforeEach(() => {
  vi.clearAllMocks();
  mock.info.mockResolvedValue({ root: '/old', availableBytes: 2 ** 30 });
  mock.open.mockResolvedValue('/new');
});
it('switches displayed storage after a verified move and explains retained originals', async () => {
  let complete!: (value: { root: string; availableBytes: number }) => void;
  mock.move.mockReturnValue(new Promise(resolve => { complete = resolve; }));
  const onMoved = vi.fn(async () => {});
  render(<ManagedStorage busy={false} onMoved={onMoved} />);
  await screen.findByText('/old');
  fireEvent.click(screen.getByRole('button', { name: 'Change storage location' }));
  await waitFor(() => expect(mock.move).toHaveBeenCalledWith('/new'));
  expect(screen.getByRole('button', { name: 'Copying and verifying storage…' })).toBeDisabled();
  complete({ root: '/new', availableBytes: 2 ** 30 });
  expect(await screen.findByText('/new')).toBeVisible();
  expect(screen.getByRole('status')).toHaveTextContent('original files are retained');
  expect(onMoved).toHaveBeenCalledOnce();
});
it('retains the old location after a failed migration and allows retry', async () => {
  mock.move.mockRejectedValue(new Error('Not enough destination space'));
  render(<ManagedStorage busy={false} />);
  await screen.findByText('/old');
  fireEvent.click(screen.getByRole('button', { name: 'Change storage location' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Not enough destination space');
  expect(screen.getByText('/old')).toBeVisible();
  expect(screen.getByRole('button', { name: 'Change storage location' })).toBeEnabled();
});
