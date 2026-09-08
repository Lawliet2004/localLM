import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { RuntimeForm } from './RuntimeForm';
import { defaultRuntimeConfig } from '../lib/types';

describe('runtime settings', () => {
  it('saves CPU-only placement without changing the context window', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    await userEvent.selectOptions(screen.getByLabelText('Compute device'), 'cpu');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ gpuLayers: 0, contextLength: 8192 }));
  });

  it('prevents saving quantized value cache without Flash Attention', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    await userEvent.click(screen.getByLabelText('Flash Attention'));
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Flash Attention');
  });

  it('does not let users submit a micro batch larger than the batch', async () => {
    const onSave = vi.fn();
    render(<RuntimeForm initial={defaultRuntimeConfig} onSave={onSave} busy={false} />);
    const input = screen.getByLabelText('Micro batch size');
    await userEvent.clear(input);
    await userEvent.type(input, '1024');
    await userEvent.click(screen.getByRole('button', { name: 'Save configuration' }));
    expect(onSave).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent('Micro batch');
  });
});
