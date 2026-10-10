import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
import { DEFAULT_APPEARANCE } from '../lib/appearance';
import { defaultRuntimeConfig } from '../lib/types';

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ isMaximized: async () => false, onResized: async () => () => {} }) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn(), open: vi.fn(), confirm: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e), api: new Proxy({
  bootstrap: async () => ({
    conversations: [], config: defaultRuntimeConfig,
    preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 100, systemPrompt: '' },
    runtime: { phase: 'stopped', message: '', modelPath: null },
    providers: [], preferredModel: { providerId: null, modelId: '' },
    rememberedTools: { sources: [], tools: [] },
  }),
  workspaceIndex: async () => ({ projects: [], tasks: {} }),
  tasksList: async () => [],
} as Record<string | symbol, unknown>, { get: (target, prop) => prop in target ? target[prop] : async () => undefined }) }));

async function openAppearance() {
  render(<App />);
  await screen.findByRole('button', { name: 'Settings' });
  await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
  await userEvent.click(await screen.findByRole('button', { name: 'Appearance' }));
  await screen.findByRole('heading', { name: 'Appearance' });
}

beforeEach(() => {
  localStorage.clear();
  const el = document.documentElement;
  for (const key of ['theme', 'stock', 'accent', 'density', 'userMessage', 'motion', 'contrast', 'hideStats', 'hideChatTimes']) delete el.dataset[key];
  el.removeAttribute('style');
});

describe('appearance settings', () => {
  it('applies mode, stock and safelight choices to the document immediately', async () => {
    await openAppearance();
    await userEvent.click(screen.getByRole('radio', { name: 'Light' }));
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(document.documentElement.dataset.stock).toBe('fiber');
    await userEvent.click(screen.getByRole('radio', { name: 'Ruby' }));
    expect(document.documentElement.dataset.accent).toBe('ruby');
    await userEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('pushes range and segmented choices onto <html> as variables', async () => {
    await openAppearance();
    fireEvent.change(screen.getByLabelText('Chat text size'), { target: { value: '17' } });
    expect(document.documentElement.style.getPropertyValue('--chat-font-size')).toBe('17px');
    fireEvent.change(screen.getByLabelText('Chat line height'), { target: { value: '1.6' } });
    expect(document.documentElement.style.getPropertyValue('--chat-line-height')).toBe('1.6');
    await userEvent.click(screen.getByRole('radio', { name: 'Wide' }));
    expect(document.documentElement.style.getPropertyValue('--chat-width')).toBe('920px');
    await userEvent.click(screen.getByRole('radio', { name: 'Plain' }));
    expect(document.documentElement.dataset.userMessage).toBe('plain');
  });

  it('hides speed stats and chat times through data flags', async () => {
    await openAppearance();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Show speed stats' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Show chat times' }));
    expect(document.documentElement).toHaveAttribute('data-hide-stats');
    expect(document.documentElement).toHaveAttribute('data-hide-chat-times');
  });

  it('persists choices and restores defaults on reset', async () => {
    await openAppearance();
    const reset = screen.getByRole('button', { name: 'Reset to defaults' });
    expect(reset).toBeDisabled();
    await userEvent.click(screen.getByRole('radio', { name: 'Light' }));
    await userEvent.click(screen.getByRole('radio', { name: 'Pitch' }));
    await userEvent.click(screen.getByRole('radio', { name: 'Cyan' }));
    expect(reset).toBeEnabled();
    expect(JSON.parse(localStorage.getItem('locallm-appearance')!)).toMatchObject({ theme: 'light', darkStock: 'pitch', accent: 'cyan' });
    expect(localStorage.getItem('locallm-theme')).toBe('light');
    await userEvent.click(reset);
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(document.documentElement.dataset.accent).toBe('amber');
    expect(JSON.parse(localStorage.getItem('locallm-appearance')!)).toEqual(DEFAULT_APPEARANCE);
  });
});
