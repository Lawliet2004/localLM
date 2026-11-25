import { act, renderHook } from '@testing-library/react';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { APPEARANCE_STORAGE_KEY, applyAppearance, contrastRatio, defaultAppearance, loadAppearance, normalizeAppearance } from './appearance';
import { useAppearance } from './useAppearance';

beforeEach(() => { localStorage.clear(); });
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('appearance preferences', () => {
  it('migrates an existing theme and recovers from corrupt saved preferences', () => {
    localStorage.setItem('locallm-theme', 'dark');
    expect(loadAppearance()).toEqual({ ...defaultAppearance, theme: 'dark' });
    localStorage.setItem(APPEARANCE_STORAGE_KEY, '{broken json');
    expect(loadAppearance().theme).toBe('dark');
  });

  it('rejects invalid values independently and retains supported preferences', () => {
    expect(normalizeAppearance({ theme: 'dark', accent: 'invalid', chatFontSize: 200, codeFontSize: -1, density: 'compact', motion: true, chatWidth: '__proto__' }))
      .toEqual({ ...defaultAppearance, theme: 'dark', density: 'compact' });
    expect(normalizeAppearance(null)).toEqual(defaultAppearance);
  });

  it('restores the original palette after an alternate accent and supports dark accents', () => {
    applyAppearance({ ...defaultAppearance, accent: 'blue' }, 'dark');
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#a0bff0');
    applyAppearance({ ...defaultAppearance });
    expect(document.documentElement.style.getPropertyValue('--accent')).toBe('');
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
  });

  it('tints app surfaces in both modes and retains readable text', () => {
    const style = document.documentElement.style;
    for (const theme of ['light', 'dark'] as const) {
      applyAppearance({ ...defaultAppearance, accent: 'rose' }, theme);
      const roseSurface = style.getPropertyValue('--surface');
      const roseSidebar = style.getPropertyValue('--sidebar');
      applyAppearance({ ...defaultAppearance, accent: 'blue' }, theme);
      expect(style.getPropertyValue('--surface')).not.toBe(roseSurface);
      expect(style.getPropertyValue('--sidebar')).not.toBe(roseSidebar);
      expect(contrastRatio(style.getPropertyValue('--secondary'), style.getPropertyValue('--surface'))).toBeGreaterThan(4.5);
    }
    applyAppearance({ ...defaultAppearance });
    expect(style.getPropertyValue('--surface')).toBe('');
  });

  it('discards custom wallpaper values that could inject a URL', () => {
    expect(normalizeAppearance({ homeWallpaper: 'custom', customWallpaper: 'https://example.com/photo.jpg' }))
      .toMatchObject({ homeWallpaper: 'none', customWallpaper: '' });
  });

  it('follows system appearance changes and keeps explicit theme choices fixed', () => {
    const listeners = new Set<() => void>();
    const media = { matches: false, addEventListener: (_: string, fn: () => void) => listeners.add(fn), removeEventListener: (_: string, fn: () => void) => listeners.delete(fn) };
    vi.stubGlobal('matchMedia', () => media);
    const { result, unmount } = renderHook(useAppearance);
    act(() => result.current.update({ theme: 'system' }));
    expect(result.current.theme).toBe('light');
    act(() => { media.matches = true; listeners.forEach(listener => listener()); });
    expect(result.current.theme).toBe('dark');
    expect(result.current.preferences.theme).toBe('system');
    act(() => result.current.update({ theme: 'light' }));
    expect(result.current.theme).toBe('light');
    unmount();
    expect(listeners.size).toBe(0);
  });

  it('saves all controls together, restores them on restart, and resets every preference', () => {
    const first = renderHook(useAppearance);
    act(() => first.result.current.update({ theme: 'dark', chatFontSize: 20, codeFontSize: 18, accent: 'rose', chatFont: 'serif', chatWidth: 'full', density: 'compact', motion: 'reduced' }));
    const saved = first.result.current.preferences;
    first.unmount();
    const second = renderHook(useAppearance);
    expect(second.result.current.preferences).toEqual(saved);
    act(() => second.result.current.reset());
    expect(loadAppearance()).toEqual(defaultAppearance);
    expect(document.documentElement).toHaveAttribute('data-density', 'comfortable');
  });

  it('keeps changes usable and reports storage failure without claiming they were saved', () => {
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Storage blocked'); });
    const { result } = renderHook(useAppearance);
    act(() => result.current.update({ theme: 'dark' }));
    expect(result.current.theme).toBe('dark');
    expect(result.current.saveError).toContain('could not be saved');
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
  });

  it('handles unavailable storage at startup and synchronizes other windows', () => {
    const blocked = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('Storage blocked'); });
    expect(loadAppearance()).toEqual(defaultAppearance);
    blocked.mockRestore();
    const { result } = renderHook(useAppearance);
    act(() => {
      localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify({ ...defaultAppearance, theme: 'dark', chatFontSize: 18 }));
      window.dispatchEvent(new StorageEvent('storage', { key: APPEARANCE_STORAGE_KEY }));
    });
    expect(result.current.theme).toBe('dark');
    expect(result.current.preferences.chatFontSize).toBe(18);
  });
});
