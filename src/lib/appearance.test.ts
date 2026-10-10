import { beforeEach, describe, expect, it } from 'vitest';
import { applyAppearance, DEFAULT_APPEARANCE, loadAppearance, saveAppearance } from './appearance';

beforeEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.removeAttribute('data-stock');
  document.documentElement.removeAttribute('data-accent');
  document.documentElement.removeAttribute('data-hide-stats');
  document.documentElement.removeAttribute('data-hide-chat-times');
});

describe('loadAppearance', () => {
  it('returns defaults when nothing is stored', () => {
    expect(loadAppearance()).toEqual(DEFAULT_APPEARANCE);
  });

  it('returns defaults on garbage JSON', () => {
    localStorage.setItem('locallm-appearance', '{not json');
    expect(loadAppearance()).toEqual(DEFAULT_APPEARANCE);
    localStorage.setItem('locallm-appearance', '42');
    expect(loadAppearance()).toEqual(DEFAULT_APPEARANCE);
  });

  it('drops invalid fields and keeps valid ones', () => {
    localStorage.setItem('locallm-appearance', JSON.stringify({
      theme: 'sepia', darkStock: 'pitch', accent: 'neon', customAccent: 'red',
      chatFontSize: 'big', codeLigatures: 'yes', userMessage: 'plain',
    }));
    const a = loadAppearance();
    expect(a.theme).toBe('dark');
    expect(a.darkStock).toBe('pitch');
    expect(a.accent).toBe('amber');
    expect(a.customAccent).toBe('#f08a3c');
    expect(a.chatFontSize).toBe(15);
    expect(a.codeLigatures).toBe(true);
    expect(a.userMessage).toBe('plain');
  });

  it('migrates the legacy locallm-theme key when no appearance key exists', () => {
    localStorage.setItem('locallm-theme', 'light');
    expect(loadAppearance().theme).toBe('light');
    // A saved appearance object wins over the legacy key.
    localStorage.setItem('locallm-appearance', JSON.stringify({ theme: 'dark' }));
    expect(loadAppearance().theme).toBe('dark');
  });

  it('clamps out-of-range numeric settings', () => {
    localStorage.setItem('locallm-appearance', JSON.stringify({ chatFontSize: 99, chatLineHeight: 4 }));
    const a = loadAppearance();
    expect(a.chatFontSize).toBe(19);
    expect(a.chatLineHeight).toBe(1.9);
    localStorage.setItem('locallm-appearance', JSON.stringify({ chatFontSize: 4, chatLineHeight: 1 }));
    const b = loadAppearance();
    expect(b.chatFontSize).toBe(13);
    expect(b.chatLineHeight).toBe(1.45);
  });
});

describe('saveAppearance', () => {
  it('persists the appearance and mirrors the theme to the legacy key', () => {
    saveAppearance({ ...DEFAULT_APPEARANCE, theme: 'light', accent: 'ruby' });
    expect(JSON.parse(localStorage.getItem('locallm-appearance')!)).toMatchObject({ theme: 'light', accent: 'ruby' });
    expect(localStorage.getItem('locallm-theme')).toBe('light');
  });
});

describe('applyAppearance', () => {
  it('sets data-stock from the resolved mode', () => {
    applyAppearance({ ...DEFAULT_APPEARANCE, darkStock: 'ink', lightStock: 'cool' }, 'dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(document.documentElement.dataset.stock).toBe('ink');
    applyAppearance({ ...DEFAULT_APPEARANCE, darkStock: 'ink', lightStock: 'cool' }, 'light');
    expect(document.documentElement.dataset.stock).toBe('cool');
  });

  it('sets and removes the hide flags', () => {
    applyAppearance({ ...DEFAULT_APPEARANCE, showStats: false, showChatTimes: false }, 'dark');
    expect(document.documentElement).toHaveAttribute('data-hide-stats');
    expect(document.documentElement).toHaveAttribute('data-hide-chat-times');
    applyAppearance(DEFAULT_APPEARANCE, 'dark');
    expect(document.documentElement).not.toHaveAttribute('data-hide-stats');
    expect(document.documentElement).not.toHaveAttribute('data-hide-chat-times');
  });

  it('writes scale variables on the root element', () => {
    applyAppearance({ ...DEFAULT_APPEARANCE, chatFontSize: 17, accent: 'cyan' }, 'dark');
    const style = document.documentElement.style;
    expect(style.getPropertyValue('--chat-font-size')).toBe('17px');
    expect(style.getPropertyValue('--chat-width')).toBe('760px');
    expect(document.documentElement.dataset.accent).toBe('cyan');
  });
});
