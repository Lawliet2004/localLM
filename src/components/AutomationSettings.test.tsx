import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AutomationSettings } from './AutomationSettings';
const f = vi.hoisted(() => ({ native: true, get: vi.fn(), save: vi.fn(), install: vi.fn(), check: vi.fn(), reset: vi.fn() }));
vi.mock('../lib/api', () => ({ get nativeAvailable() { return f.native; }, errorMessage: String, api: { getAutomationConfig: f.get, saveAutomationConfig: f.save, installBrowserEngine: f.install, automationDiagnostics: f.check, resetAutomation: f.reset } }));
const defaults = { browserEnabled: false, computerEnabled: false, browserPackagePath: '', browserExecutablePath: '', browserChannel: 'chrome', headless: false };
beforeEach(() => { vi.resetAllMocks(); f.native = true; f.get.mockResolvedValue(defaults); f.save.mockResolvedValue(undefined); f.reset.mockResolvedValue(undefined); });

describe('automation settings', () => {
  it('saves browser choices while preserving saved computer settings', async () => {
    f.get.mockResolvedValue({ ...defaults, computerEnabled: true });
    render(<AutomationSettings mode="browser" />); await act(async () => {});
    expect(screen.getByRole('heading', { name: 'Browser Use' })).toBeInTheDocument();
    expect(screen.queryByLabelText('Enable computer use')).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Enable browser use'));
    fireEvent.change(screen.getByLabelText('Browser'), { target: { value: 'msedge' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save browser settings' }));
    await waitFor(() => expect(f.save).toHaveBeenCalledWith({ ...defaults, browserEnabled: true, computerEnabled: true, browserChannel: 'msedge' }));
    expect(await screen.findByText(/Browser Use settings saved/)).toBeInTheDocument();
  });

  it('saves computer choices without exposing or changing browser configuration', async () => {
    const saved = { ...defaults, browserEnabled: true, browserChannel: 'msedge', headless: true, browserPackagePath: '/saved/engine', browserExecutablePath: '/saved/browser' };
    f.get.mockResolvedValue(saved);
    render(<AutomationSettings mode="computer" />); await act(async () => {});
    expect(screen.getByRole('heading', { name: 'Computer Use' })).toBeInTheDocument();
    expect(screen.queryByLabelText('Enable browser use')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Set up browser engine' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Reset browser sessions' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Enable computer use'));
    fireEvent.click(screen.getByRole('button', { name: 'Save computer settings' }));
    await waitFor(() => expect(f.save).toHaveBeenCalledWith({ ...saved, computerEnabled: true }));
    expect(await screen.findByText(/Computer Use settings saved/)).toBeInTheDocument();
  });

  it('installs the pinned engine without enabling it or saving unsent choices', async () => {
    f.install.mockResolvedValue({ ...defaults, browserPackagePath: '/managed/@playwright/mcp' });
    render(<AutomationSettings mode="browser" />); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Set up browser engine' }));
    expect(await screen.findByText(/Browser engine installed/)).toBeInTheDocument();
    expect(screen.getByLabelText('Browser engine package folder')).toHaveValue('/managed/@playwright/mcp');
    expect(screen.getByLabelText('Enable browser use')).not.toBeChecked();
    expect(f.save).not.toHaveBeenCalled();
  });

  it('reports setup failures and can reset browser sessions', async () => {
    f.install.mockRejectedValue(new Error('npm is missing'));
    render(<AutomationSettings mode="browser" />); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Set up browser engine' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('npm is missing');
    fireEvent.click(screen.getByRole('button', { name: 'Reset browser sessions' }));
    expect(await screen.findByText(/Browser sessions closed/)).toBeInTheDocument();
    expect(f.reset).toHaveBeenCalledOnce();
  });

  it('diagnoses the current browser form and clears stale results after editing', async () => {
    f.check.mockResolvedValue({ browser: { available: false, message: 'Install the engine' }, computer: { input: { available: false, reason: 'Wayland' } } });
    render(<AutomationSettings mode="browser" />); await act(async () => {});
    fireEvent.change(screen.getByLabelText('Browser engine package folder'), { target: { value: '/custom' } });
    fireEvent.click(screen.getByRole('button', { name: 'Check browser setup' }));
    expect(await screen.findByText('Install the engine')).toBeInTheDocument();
    expect(screen.queryByText('Desktop diagnostics')).not.toBeInTheDocument();
    expect(f.check).toHaveBeenCalledWith({ ...defaults, browserPackagePath: '/custom' });
    expect(f.save).not.toHaveBeenCalled();
    fireEvent.click(screen.getByLabelText('Run browser in the background'));
    expect(screen.queryByText('Install the engine')).not.toBeInTheDocument();
  });

  it('shows only desktop diagnostic results on the computer tab', async () => {
    f.check.mockResolvedValue({ browser: { available: false, message: 'Install the engine' }, computer: { input: { available: false, reason: 'Wayland input unavailable' } } });
    render(<AutomationSettings mode="computer" />); await act(async () => {});
    fireEvent.click(screen.getByRole('button', { name: 'Check computer setup' }));
    expect(await screen.findByText(/Computer setup check complete/)).toBeInTheDocument();
    expect(screen.getByText(/Wayland input unavailable/)).toBeInTheDocument();
    expect(screen.queryByText('Install the engine')).not.toBeInTheDocument();
    expect(f.save).not.toHaveBeenCalled();
  });

  it('prevents saving defaults over existing settings when loading fails', async () => {
    f.get.mockRejectedValue(new Error('Settings could not be read'));
    render(<AutomationSettings mode="computer" />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Settings could not be read');
    expect(screen.getByLabelText('Enable computer use')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Save computer settings' })).toBeDisabled();
    expect(f.save).not.toHaveBeenCalled();
  });

  it.each(['browser', 'computer'] as const)('keeps %s native actions unavailable in browser preview', mode => {
    f.native = false; render(<AutomationSettings mode={mode} />);
    expect(screen.getByRole('button', { name: `Save ${mode} settings` })).toBeDisabled();
    expect(screen.getByLabelText(`Enable ${mode} use`)).toBeDisabled();
    expect(f.get).not.toHaveBeenCalled();
  });
});
