import { useState } from 'react';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Segmented } from './Settings';

vi.mock('../lib/api', () => ({ nativeAvailable: false, errorMessage: String, api: {} }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn(), confirm: vi.fn() }));

describe('Segmented keyboard behavior', () => {
  it('arrow keys change selection, move focus, and use roving tabindex', async () => {
    const onChange = vi.fn();
    function Host() {
      const [value, setValue] = useState<'a' | 'b' | 'c'>('a');
      return <Segmented label="Mode" value={value} onChange={next => { setValue(next); onChange(next); }}
        options={[{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }, { value: 'c', label: 'C' }]} />;
    }
    render(<Host />);
    const group = screen.getByRole('radiogroup', { name: 'Mode' });
    const a = within(group).getByRole('radio', { name: 'A' });
    expect(a).toHaveAttribute('aria-checked', 'true');
    expect(a).toHaveAttribute('tabIndex', '0');
    within(group).getAllByRole('radio').slice(1).forEach(radio => expect(radio).toHaveAttribute('tabIndex', '-1'));

    a.focus();
    fireEvent.keyDown(group, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledWith('b');
    const b = within(group).getByRole('radio', { name: 'B' });
    expect(b).toHaveAttribute('aria-checked', 'true');
    expect(b).toHaveAttribute('tabIndex', '0');
    await waitFor(() => expect(b).toHaveFocus());

    fireEvent.keyDown(group, { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith('a');
    await waitFor(() => expect(within(group).getByRole('radio', { name: 'A' })).toHaveFocus());

    // Wraps around: Left from first selects the last option.
    fireEvent.keyDown(group, { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith('c');
    await waitFor(() => expect(within(group).getByRole('radio', { name: 'C' })).toHaveFocus());
  });
});
