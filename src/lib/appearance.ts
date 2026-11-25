import { homeWallpapers, validWallpaperData, wallpaperUrl, type HomeWallpaper } from './homeWallpaper';

export type ThemePreference = 'light' | 'dark' | 'system';
export type ResolvedTheme = 'light' | 'dark';

export const accentColors = {
  forest: { label: 'Forest', light: '#356349', dark: '#afc9a0', strongLight: '#254e37', strongDark: '#c6dbb9' },
  blue: { label: 'Ocean', light: '#315f9c', dark: '#a0bff0', strongLight: '#234977', strongDark: '#c1d7f7' },
  violet: { label: 'Violet', light: '#70509b', dark: '#c9b0ed', strongLight: '#55387b', strongDark: '#dfccf7' },
  rose: { label: 'Rose', light: '#9a4561', dark: '#edafc3', strongLight: '#78334b', strongDark: '#f7ccda' },
  amber: { label: 'Amber', light: '#856012', dark: '#e1c080', strongLight: '#65480b', strongDark: '#f1d8a7' },
} as const;

export const palettePresets = { classic: 'LocalLM Classic', graphite: 'Graphite', indigo: 'Indigo' } as const;

export const colorGroups = [
  { label: 'Interface colors', colors: {
    '--background': 'Workspace', '--sidebar': 'Sidebar', '--surface': 'Panels', '--input': 'Inputs',
    '--text': 'Main text', '--secondary': 'Secondary text', '--border': 'Borders', '--accent': 'App color override',
  } },
  { label: 'Code frame', colors: {
    '--code-bg': 'Code background', '--code-fg': 'Plain code', '--code-header-bg': 'Code header',
    '--code-border': 'Code border', '--code-gutter': 'Line numbers',
  } },
  { label: 'Syntax highlighting', colors: {
    '--syn-keyword': 'Keywords', '--syn-string': 'Strings', '--syn-number': 'Numbers & literals',
    '--syn-comment': 'Comments', '--syn-function': 'Functions', '--syn-type': 'Types & built-ins',
    '--syn-attr': 'Attributes & variables', '--syn-meta': 'Tags & metadata', '--syn-link': 'Links',
  } },
] as const;

type ColorGroup = typeof colorGroups[number]['colors'];
type KeysOfUnion<T> = T extends unknown ? keyof T : never;
export type PaletteToken = KeysOfUnion<ColorGroup>;
export type PaletteOverrides = Partial<Record<PaletteToken, string>>;
export type PalettePreset = keyof typeof palettePresets;
export const paletteTokens = colorGroups.flatMap(group => Object.keys(group.colors)) as PaletteToken[];
export const hexColor = /^#[0-9a-f]{6}$/i;

function normalizePalette(value: unknown): PaletteOverrides {
  const input = value && typeof value === 'object' ? value as Record<string, unknown> : {};
  const colors: PaletteOverrides = {};
  for (const token of paletteTokens) {
    const color = input[token];
    if (typeof color === 'string' && hexColor.test(color)) colors[token] = color.toLowerCase();
  }
  return colors;
}

export interface AppearancePreferences {
  theme: ThemePreference;
  accent: keyof typeof accentColors;
  chatFont: 'plex' | 'system' | 'serif';
  chatFontSize: number;
  codeFontSize: number;
  density: 'comfortable' | 'compact';
  chatWidth: 'standard' | 'wide' | 'full';
  motion: 'system' | 'reduced';
  motionSpeed: 'fast' | 'standard' | 'slow';
  palette: PalettePreset;
  homeWallpaper: HomeWallpaper;
  customWallpaper: string;
  lightColors: PaletteOverrides;
  darkColors: PaletteOverrides;
}

export const APPEARANCE_STORAGE_KEY = 'locallm-appearance';
export const defaultAppearance: Readonly<AppearancePreferences> = {
  theme: 'light', accent: 'forest', chatFont: 'plex', chatFontSize: 16,
  codeFontSize: 13, density: 'comfortable', chatWidth: 'standard', motion: 'system',
  motionSpeed: 'standard', palette: 'classic', lightColors: {}, darkColors: {}, homeWallpaper: 'none', customWallpaper: '',
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
  const customWallpaper = validWallpaperData(prefs.customWallpaper) ? prefs.customWallpaper : '';
  return {
    theme: choice(prefs.theme, ['light', 'dark', 'system'], defaultAppearance.theme),
    accent: choice(prefs.accent, Object.keys(accentColors) as (keyof typeof accentColors)[], defaultAppearance.accent),
    chatFont: choice(prefs.chatFont, ['plex', 'system', 'serif'], defaultAppearance.chatFont),
    chatFontSize: size(prefs.chatFontSize, 12, 20, defaultAppearance.chatFontSize),
    codeFontSize: size(prefs.codeFontSize, 11, 18, defaultAppearance.codeFontSize),
    density: choice(prefs.density, ['comfortable', 'compact'], defaultAppearance.density),
    chatWidth: choice(prefs.chatWidth, ['standard', 'wide', 'full'], defaultAppearance.chatWidth),
    motion: choice(prefs.motion, ['system', 'reduced'], defaultAppearance.motion),
    motionSpeed: choice(prefs.motionSpeed, ['fast', 'standard', 'slow'], defaultAppearance.motionSpeed),
    palette: choice(prefs.palette, Object.keys(palettePresets) as PalettePreset[], defaultAppearance.palette),
    homeWallpaper: choice(prefs.homeWallpaper, ['none', ...Object.keys(homeWallpapers) as (keyof typeof homeWallpapers)[],
      ...(customWallpaper ? ['custom' as const] : [])], 'none'),
    customWallpaper,
    lightColors: normalizePalette(prefs.lightColors),
    darkColors: normalizePalette(prefs.darkColors),
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
  root.dataset.motionSpeed = preferences.motionSpeed;
  root.dataset.palette = preferences.palette;
  root.dataset.chatWidth = preferences.chatWidth;
  root.dataset.chatFont = preferences.chatFont;
  const wallpaper = wallpaperUrl(preferences.homeWallpaper, preferences.customWallpaper);
  const wallpaperStyle = wallpaper ? `url("${wallpaper}")` : '';
  if (root.style.getPropertyValue('--home-wallpaper') !== wallpaperStyle) {
    if (wallpaperStyle) root.style.setProperty('--home-wallpaper', wallpaperStyle);
    else root.style.removeProperty('--home-wallpaper');
  }
  root.dataset.homeWallpaper = wallpaper ? preferences.homeWallpaper : 'none';
  const palette = accentColors[preferences.accent];
  const overrides = preferences[theme === 'dark' ? 'darkColors' : 'lightColors'];
  const customAccent = overrides['--accent'];
  const accent = customAccent ?? palette[theme];
  const strong = customAccent
    ? 'color-mix(in srgb, var(--accent) 86%, var(--text))'
    : theme === 'dark' ? palette.strongDark : palette.strongLight;
  const accentStyles = {
    '--accent': accent, '--accent-strong': strong, '--accent-2': accent, '--accent-3': accent,
    '--accent-soft': `${accent}18`, '--accent-subtle': `${accent}08`, '--focus-border': accent,
    '--button-bg': accent, '--button-text': customAccent
      ? (contrastRatio(accent, '#ffffff') >= contrastRatio(accent, '#000000') ? '#ffffff' : '#000000')
      : theme === 'dark' ? '#141417' : '#ffffff',
    '--active': `${accent}${theme === 'dark' ? '28' : '14'}`,
  };
  const useAppColor = preferences.accent !== 'forest' || !!customAccent;
  const colors: Record<string, string> = {
    ...(useAppColor ? tintApplication(accent, theme, preferences.palette) : {}),
    ...overrides,
    ...(overrides['--text'] ? { '--foreground': overrides['--text'] } : {}),
    ...(overrides['--secondary'] ? { '--muted': overrides['--secondary'], '--rail-text': overrides['--secondary'] } : {}),
    ...(overrides['--sidebar'] ? { '--rail-bg': overrides['--sidebar'] } : {}),
    ...(overrides['--surface'] ? { '--overlay': overrides['--surface'], '--elevated': overrides['--surface'] } : {}),
    ...(useAppColor ? accentStyles : {}),
  };
  const aliases = ['--foreground', '--muted', '--rail-text', '--rail-bg', '--overlay', '--elevated', '--hover', '--border-strong', '--folder'];
  for (const name of new Set([...paletteTokens, ...Object.keys(accentStyles), ...aliases])) {
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


function mixHex(base: string, tint: string, strength: number): string {
  const channels = [1, 3, 5].map(offset => {
    const from = parseInt(base.slice(offset, offset + 2), 16);
    const to = parseInt(tint.slice(offset, offset + 2), 16);
    return Math.round(from + (to - from) * strength).toString(16).padStart(2, '0');
  });
  return `#${channels.join('')}`;
}

function tintApplication(color: string, theme: ResolvedTheme, palette: PalettePreset): Record<string, string> {
  const dark = theme === 'dark';
  const anchor = palette === 'indigo' ? (dark ? '#161222' : '#f6f3ff') : dark ? '#121216' : '#ffffff';
  const strength = palette === 'graphite' ? 0.72 : 1;
  const tint = (light: number, night: number) => mixHex(anchor, color, (dark ? night : light) * strength);
  const text = mixHex(dark ? '#ffffff' : '#101016', color, 0.17);
  const secondary = mixHex(dark ? '#e2e5ed' : '#393945', color, 0.24);
  const sidebar = tint(0.19, 0.12);
  const border = tint(0.24, 0.24);
  return {
    '--background': tint(0.15, 0.075), '--sidebar': sidebar, '--rail-bg': sidebar,
    '--surface': tint(0.065, 0.10), '--input': tint(0.025, 0.14),
    '--elevated': tint(0.09, 0.17), '--overlay': tint(0.11, 0.16),
    '--text': text, '--foreground': text, '--secondary': secondary, '--muted': secondary, '--rail-text': secondary,
    '--border': border, '--border-strong': tint(0.39, 0.43), '--hover': tint(0.18, 0.21),
    '--folder': color, '--code-bg': tint(0.09, 0.105), '--code-fg': text,
    '--code-header-bg': tint(0.17, 0.17), '--code-border': border, '--code-gutter': secondary,
  };
}

function luminance(hex: string): number {
  const rgb = [1, 3, 5].map(offset => {
    const channel = parseInt(hex.slice(offset, offset + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
}

export function contrastRatio(first: string, second: string): number {
  const a = luminance(first);
  const b = luminance(second);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
