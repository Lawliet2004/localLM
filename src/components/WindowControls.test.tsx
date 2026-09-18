import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { WindowControls } from './WindowControls';

const controls = vi.hoisted(() => ({ minimize: vi.fn(), toggleMaximize: vi.fn(), close: vi.fn(), isMaximized: vi.fn().mockResolvedValue(false), onResized: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => controls }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String }));

it('routes window controls to native window actions', async () => {
  render(<WindowControls onError={vi.fn()} />);
  await userEvent.click(screen.getByRole('button', { name: 'Minimize window' }));
  await userEvent.click(screen.getByRole('button', { name: 'Maximize window' }));
  await userEvent.click(screen.getByRole('button', { name: 'Close window' }));
  expect(controls.minimize).toHaveBeenCalledOnce();
  expect(controls.toggleMaximize).toHaveBeenCalledOnce();
  expect(controls.close).toHaveBeenCalledOnce();
});
