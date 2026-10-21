import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { NativeExecutionSettings } from './NativeExecutionSettings';
const fixtures = vi.hoisted(() => ({ save: vi.fn(), get: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  getExecutionConfig: fixtures.get, saveExecutionConfig: fixtures.save,
} }));
beforeEach(() => {
  fixtures.get.mockResolvedValue({ pythonPath: '/usr/bin/python3', nodePath: '/usr/bin/node', powershellPath: '', bashPath: '/bin/bash', bash: { enabled: true, timeoutSeconds: 60, workingDirectory: '', environment: {}, alwaysAsk: false } });
  fixtures.save.mockReset().mockResolvedValue(undefined);
});
describe('native execution settings', () => {
  it('persists Python policy and keeps the existing shell and Node configuration', async () => {
    render(<NativeExecutionSettings kind="python" />);
    await act(async () => {});
    fireEvent.click(screen.getByLabelText('Enable Python execution'));
    fireEvent.change(screen.getByLabelText('Virtual environment folder'), { target: { value: '/project/.venv' } });
    fireEvent.change(screen.getByLabelText('Timeout (seconds)'), { target: { value: '30' } });
    fireEvent.click(screen.getByLabelText('Always request approval, including in Full access'));
    fireEvent.change(screen.getByLabelText('Environment variables'), { target: { value: '{"MODE":"test"}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save python settings' }));
    await waitFor(() => expect(fixtures.save).toHaveBeenCalledWith(expect.objectContaining({
      bashPath: '/bin/bash', nodePath: '/usr/bin/node', pythonVenv: '/project/.venv',
      python: expect.objectContaining({ enabled: false, timeoutSeconds: 30, alwaysAsk: true, environment: { MODE: 'test' } }),
    })));
    expect(await screen.findByRole('status')).toHaveTextContent('Python settings saved');
  });
  it('rejects invalid environment without calling the backend or claiming a save', async () => {
    render(<NativeExecutionSettings kind="bash" />);
    await act(async () => {});
    fireEvent.change(screen.getByLabelText('Environment variables'), { target: { value: '{"NUMBER":42}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save bash settings' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Environment must be a JSON object with string values');
    expect(fixtures.save).not.toHaveBeenCalled();
  });
});
