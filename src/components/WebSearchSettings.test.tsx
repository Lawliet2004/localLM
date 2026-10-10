import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { Execution } from './Execution';
const fixtures = vi.hoisted(() => ({ save: vi.fn(), health: vi.fn(), config: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  getWebSearchConfig: fixtures.config,
  saveWebSearchConfig: fixtures.save, webSearchHealth: fixtures.health,
  getExecutionConfig: async () => ({ pythonPath: '', nodePath: '', powershellPath: '' }),
  hasDaytonaKey: async () => false, pendingDaytonaOperations: async () => [],
} }));

beforeEach(() => {
  fixtures.config.mockResolvedValue({ provider: 'google', searxngBaseUrl: 'http://127.0.0.1:8080', hasGoogleApiKey: true, googleCxId: 'cx', searchFallbackEnabled: true });
});

it('keeps saved credentials out of the input and makes clearing explicit', async () => {
  fixtures.save.mockResolvedValue(undefined);
  render(<Execution section="web-search" />);
  const key = await screen.findByLabelText('Google API key');
  expect(key).toHaveValue(''); expect(key).toHaveAttribute('placeholder', 'Saved key · leave blank to keep');
  fireEvent.click(screen.getByRole('button', { name: 'Save search settings' }));
  await waitFor(() => expect(fixtures.save).toHaveBeenLastCalledWith(expect.objectContaining({ hasGoogleApiKey: true }), '', false));
  await screen.findByText('Search settings saved.');
  fireEvent.click(screen.getByLabelText('Clear saved Google API key on save'));
  fireEvent.click(screen.getByRole('button', { name: 'Save search settings' }));
  await waitFor(() => expect(fixtures.save).toHaveBeenLastCalledWith(expect.any(Object), '', true));
});

it('labels a configured Google connection as untested', async () => {
  fixtures.health.mockResolvedValue({ search: 'configured_not_tested' });
  render(<Execution section="web-search" />);
  fireEvent.click(await screen.findByRole('button', { name: 'Check connection' }));
  expect(await screen.findByText('Google is configured; connection was not tested.')).toBeInTheDocument();
});

it('explains how to recover an unavailable search provider', async () => {
  fixtures.health.mockResolvedValue({ search: 'unreachable' });
  render(<Execution section="web-search" />);
  fireEvent.click(await screen.findByRole('button', { name: 'Check connection' }));
  expect(await screen.findByText('Search provider is unavailable. Check that the service is running and the saved URL is correct. Save any changes before checking again.')).toBeInTheDocument();
});

it('explains why an enabled Google fallback cannot supply results', async () => {
  fixtures.config.mockResolvedValue({ provider: 'searxng', searxngBaseUrl: 'http://127.0.0.1:8080', hasGoogleApiKey: false, googleCxId: '', searchFallbackEnabled: true });
  fixtures.health.mockResolvedValue({ search: 'unreachable', google: 'not_configured' });
  render(<Execution section="web-search" />);
  await screen.findByRole('textbox', { name: 'SearXNG base URL' });
  fireEvent.click(await screen.findByRole('button', { name: 'Check connection' }));
  expect(await screen.findByText(/Google fallback also needs an API key and a Custom Search Engine ID/)).toBeInTheDocument();
});
