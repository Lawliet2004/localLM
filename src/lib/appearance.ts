export type ThemePreference = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

export const accentColors = {
  forest: { label: 'Forest', light: '#356349', dark: '#afc9a0', strongLight: '#254e37', strongDark: '#c6dbb9' },
  blue: { label: 'Ocean', light: '#315f9c', dark: '#a0bff0', strongLight: '#234977', strongDark: '#c1d7f7' },
  violet: { label: 'Violet', light: '#70509b', dark: '#c9b0ed', strongLight: '#55387b', strongDark: '#dfccf7' },
  rose: { label: 'Rose', light: '#9a4561', dark: '#edafc3', strongLight: '#78334b', strongDark: '#f7ccda' },
  amber: { label: 'Amber', light: '#856012', dark: '#e1c080', strongLight: '#65480b', strongDark: '#f1d8a7' },
} as const;

export interface AppearancePreferences {
  theme: ThemePreference;
  accent: keyof typeof accentColors;
  chatFont: 'plex' | 'system' | 'serif';
  chatFontSize: number;
  codeFontSize: number;
  density: 'comfortable' | 'compact';
  chatWidth: 'standard' | 'wide' | 'full';
  motion: 'system' | 'reduced';
}

export const APPEARANCE_STORAGE_KEY = 'locallm-appearance';
export const defaultAppearance: Readonly<AppearancePreferences> = {
  theme: 'light', accent: 'forest', chatFont: 'plex', chatFontSize: 16,
  codeFontSize: 13, density: 'comfortable', chatWidth: 'standard', motion: 'system',
};

function choice<T extends string>(value: unknown, options: readonly T[], fallback: T): T {
  return options.includes(value as T) ? value as T : fallback;
}

function size(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max ? value : fallback;
}

/** Validate each preference independently so old or damaged values stay recoverable. */
export function normalizeAppearance(value: unknown): AppearancePreferences {
  const prefs = value && typeof value === 'object' ? value as Record<string, unknown> : {};
  return {
    theme: choice(prefs.theme, ['light', 'dark', 'system'], defaultAppearance.theme),
    accent: choice(prefs.accent, Object.keys(accentColors) as (keyof typeof accentColors)[], defaultAppearance.accent),
    chatFont: choice(prefs.chatFont, ['plex', 'system', 'serif'], defaultAppearance.chatFont),
    chatFontSize: size(prefs.chatFontSize, 12, 20, defaultAppearance.chatFontSize),
    codeFontSize: size(prefs.codeFontSize, 11, 18, defaultAppearance.codeFontSize),
    density: choice(prefs.density, ['comfortable', 'compact'], defaultAppearance.density),
    chatWidth: choice(prefs.chatWidth, ['standard', 'wide', 'full'], defaultAppearance.chatWidth),
    motion: choice(prefs.motion, ['system', 'reduced'], defaultAppearance.motion),
  };
}

export function loadAppearance(): AppearancePreferences {
  try {
    const saved = localStorage.getItem(APPEARANCE_STORAGE_KEY);
    if (saved) {
      try { return normalizeAppearance(JSON.parse(saved)); } catch { /* Fall back to the previous theme preference. */ }
    }
    return normalizeAppearance({ theme: localStorage.getItem('locallm-theme') });
  } catch { return { ...defaultAppearance }; }
}

export function systemPrefersDark(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

export function prefersReducedMotion(): boolean {
  return document.documentElement.dataset.motion === 'reduced' ||
    (typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
}

export function resolveTheme(theme: ThemePreference, dark = systemPrefersDark()): ResolvedTheme {
  return theme === 'system' ? dark ? 'dark' : 'light' : theme;
}

export function applyAppearance(preferences: AppearancePreferences, theme = resolveTheme(preferences.theme)): void {
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.dataset.accent = preferences.accent;
  root.dataset.density = preferences.density;
  root.dataset.motion = preferences.motion;
  root.dataset.chatWidth = preferences.chatWidth;
  const palette = accentColors[preferences.accent];
  const accent = palette[theme];
  const strong = theme === 'dark' ? palette.strongDark : palette.strongLight;
  // Forest inherits the established tokens. Alternate accents change state colors only.
  const colors: Record<string, string> = preferences.accent === 'forest' ? {} : {
    '--accent': accent, '--accent-strong': strong, '--accent-2': accent, '--accent-3': accent,
    '--accent-soft': `${accent}18`, '--accent-subtle': `${accent}08`, '--focus-border': accent,
    '--button-bg': accent, '--button-text': theme === 'dark' ? '#202b21' : '#ffffff',
    '--active': `${accent}${theme === 'dark' ? '28' : '14'}`,
  };
  for (const name of ['--accent', '--accent-strong', '--accent-2', '--accent-3', '--accent-soft', '--accent-subtle', '--focus-border', '--button-bg', '--button-text', '--active']) {
    if (colors[name]) root.style.setProperty(name, colors[name]);
    else root.style.removeProperty(name);
  }
  const fonts = {
    plex: "'IBM Plex Sans', 'Segoe UI', sans-serif",
    system: 'system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
    serif: "'IBM Plex Serif', Georgia, serif",
  };
  root.style.setProperty('--chat-font', fonts[preferences.chatFont]);
  root.style.setProperty('--chat-font-size', `${preferences.chatFontSize}px`);
  root.style.setProperty('--code-font-size', `${preferences.codeFontSize}px`);
  root.style.setProperty('--chat-max-width', preferences.chatWidth === 'standard' ? '824px' : preferences.chatWidth === 'wide' ? '1080px' : '100%');
}
