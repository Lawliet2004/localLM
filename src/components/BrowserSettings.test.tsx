import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { BrowserSettings } from './Settings';

const save = vi.hoisted(() => vi.fn(async (_config: { mode: string; maxActions: number }) => {}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: String,
  api: { getBrowserConfig: async () => ({ mode: 'auto', maxActions: 12 }), saveBrowserConfig: save },
}));

describe('BrowserSettings', () => {
  it('saves the mode and a clamped per-reply action limit', async () => {
    render(<BrowserSettings />);
    const limit = await screen.findByDisplayValue('12');
    await userEvent.click(screen.getByRole('radio', { name: /Off/ }));
    expect(save).toHaveBeenLastCalledWith({ mode: 'off', maxActions: 12 });
    expect(limit).toBeDisabled();

    await userEvent.click(screen.getByRole('radio', { name: /Prefer the browser/ }));
    fireEvent.change(limit, { target: { value: '500' } });
    fireEvent.blur(limit);
    // Writes are serialized: the limit write lands after the mode write.
    await vi.waitFor(() => expect(save).toHaveBeenLastCalledWith({ mode: 'preferred', maxActions: 50 }));
    expect(await screen.findByText('Saved. Applies from your next message.')).toBeVisible();
  });

  it('reverts the selection to the saved value when a save fails', async () => {
    save.mockRejectedValueOnce(new Error('backend gone'));
    render(<BrowserSettings />);
    await screen.findByDisplayValue('12');
    await userEvent.click(screen.getByRole('radio', { name: /Prefer the browser/ }));
    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('Not saved: Error: backend gone. Showing your saved setting.');
    expect(screen.getByRole('radio', { name: /Off/ })).not.toBeChecked();
    expect(screen.getByRole('radio', { name: /Automatic/ })).toBeChecked();
  });

  it('serializes rapid changes in order and only the last completion shows', async () => {
    const calls: { mode?: string; maxActions?: number }[] = [];
    const pending: (() => void)[] = [];
    save.mockImplementation(config => { calls.push(config); return new Promise<void>(done => pending.push(done)); });
    render(<BrowserSettings />);
    await screen.findByDisplayValue('12');
    await userEvent.click(screen.getByRole('radio', { name: /Prefer the browser/ }));
    await userEvent.click(screen.getByRole('radio', { name: /Off/ }));
    expect(calls.length).toBe(1);
    pending[0]();
    await vi.waitFor(() => expect(calls.length).toBe(2));
    expect(calls[0]).toEqual({ mode: 'preferred', maxActions: 12 });
    expect(calls[1]).toEqual({ mode: 'off', maxActions: 12 });
    pending[1]();
    expect(await screen.findByText('Saved. Applies from your next message.')).toBeVisible();
  });
});
