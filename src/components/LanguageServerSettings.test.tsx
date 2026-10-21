import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LanguageServerSettings } from './LanguageServerSettings';
import { settingsItems } from './SettingsNavigation';
const fixtures = vi.hoisted(() => ({ get: vi.fn(), save: vi.fn(), test: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: { getLspConfig: fixtures.get, saveLspConfig: fixtures.save, testLspServer: fixtures.test } }));
const server = { id: 'typescript', languageId: 'typescript', command: '/usr/bin/typescript-language-server', enabled: true, arguments: ['--stdio'], environment: { MODE: 'test' }, initializationOptions: { preferences: {} }, settings: { answer: 42 }, timeoutSeconds: 30 };
beforeEach(() => {
  fixtures.get.mockReset().mockResolvedValue({ enabled: true, alwaysAsk: false, servers: [server] });
  fixtures.save.mockReset().mockResolvedValue(undefined); fixtures.test.mockReset().mockResolvedValue({ ready: true, message: 'Language server initialized.' });
});
async function loaded(root: string | undefined = '/project') { render(<LanguageServerSettings root={root} />); await act(async () => {}); }
const save = () => fireEvent.click(screen.getByRole('button', { name: 'Save language server settings' }));
describe('language server settings', () => {
  it('appears immediately after Python in Execution settings', () => {
    const index = settingsItems.findIndex(item => item.page === 'python');
    expect(settingsItems[index + 1]).toMatchObject({ page: 'language-servers', label: 'Language servers', group: 'Execution', sub: true });
  });
  it('saves policy and all server fields without dropping advanced configuration', async () => {
    await loaded();
    fireEvent.click(screen.getByLabelText('Always request approval'));
    fireEvent.change(screen.getByLabelText('Request timeout (seconds)'), { target: { value: '15' } });
    save();
    await waitFor(() => expect(fixtures.save).toHaveBeenCalledWith({ enabled: true, alwaysAsk: true, servers: [{ ...server, timeoutSeconds: 15 }] }));
    expect(await screen.findByText('Language server settings saved. New tool calls use these settings.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Test typescript' })).toBeEnabled();
  });
  it('rejects invalid JSON and non-string arguments before calling the backend', async () => {
    await loaded();
    fireEvent.change(screen.getByLabelText('Arguments'), { target: { value: '[' } }); save();
    expect(await screen.findByRole('alert')).toHaveTextContent('arguments must be valid JSON');
    fireEvent.change(screen.getByLabelText('Arguments'), { target: { value: '[42]' } }); save();
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('arguments must be a JSON array of strings'));
    expect(fixtures.save).not.toHaveBeenCalled();
  });
  it('retains failed saves as dirty and never tests unsaved configuration', async () => {
    await loaded();
    fireEvent.change(screen.getByLabelText('Server executable'), { target: { value: '/new/server' } });
    expect(screen.getByRole('button', { name: 'Test typescript' })).toBeDisabled();
    fixtures.save.mockRejectedValueOnce(new Error('Executable configuration rejected.'));
    save(); expect(await screen.findByRole('alert')).toHaveTextContent('Executable configuration rejected');
    expect(screen.getByText('Unsaved changes.')).toBeInTheDocument();
    expect(fixtures.test).not.toHaveBeenCalled();
    expect(screen.queryByText(/settings saved/)).not.toBeInTheDocument();
  });
  it('tests only saved enabled configuration in the selected workspace and reports failures', async () => {
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'Test typescript' }));
    await waitFor(() => expect(fixtures.test).toHaveBeenCalledWith('typescript', '/project'));
    expect(await screen.findByText('Language server initialized.')).toBeInTheDocument();
    fixtures.test.mockResolvedValueOnce({ ready: false, message: 'Server rejected initialization.' });
    fireEvent.click(screen.getByRole('button', { name: 'Test typescript' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Server rejected initialization');
    expect(screen.queryByText('Language server initialized.')).not.toBeInTheDocument();
  });
  it('requires a workspace to test and explains how to choose one', async () => {
    render(<LanguageServerSettings />); await act(async () => {});
    expect(screen.getByRole('button', { name: 'Test typescript' })).toBeDisabled();
    expect(screen.getByText(/Choose a workspace in the conversation/)).toBeInTheDocument();
  });
  it('keeps failed configuration loads explicit and disables editing', async () => {
    fixtures.get.mockRejectedValueOnce(new Error('Invalid stored configuration.'));
    await loaded();
    expect(screen.getByRole('alert')).toHaveTextContent('Invalid stored configuration');
    expect(screen.getByRole('button', { name: 'Add server' })).toBeDisabled();
    expect(fixtures.save).not.toHaveBeenCalled();
  });
  it('adds unique servers, caps the list, and removes only the chosen server', async () => {
    fixtures.get.mockResolvedValueOnce({ enabled: true, alwaysAsk: false, servers: Array.from({ length: 7 }, (_, i) => ({ ...server, id: `server-${i + 1}` })) });
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));
    expect(screen.getAllByLabelText('Server ID').map(input => (input as HTMLInputElement).value)).toEqual(Array.from({ length: 8 }, (_, i) => `server-${i + 1}`));
    expect(screen.getByRole('button', { name: 'Add server' })).toBeDisabled();
    fireEvent.click(screen.getByRole('button', { name: 'Remove server-3' }));
    expect(screen.getAllByLabelText('Server ID').map(input => (input as HTMLInputElement).value)).toEqual(['server-1', 'server-2', 'server-4', 'server-5', 'server-6', 'server-7', 'server-8']);
    expect(screen.getByRole('button', { name: 'Add server' })).toBeEnabled();
  });
});
