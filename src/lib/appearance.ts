/* Appearance model: persisted choices, validation, and DOM application.
   Every field maps to a data-* attribute or CSS custom property on <html>,
   so changes apply live with no reload. */

export type ThemeChoice = 'dark' | 'light' | 'system';
export type DarkStock = 'umber' | 'graphite' | 'ink' | 'pitch';
export type LightStock = 'fiber' | 'glossy' | 'cool';
export type Safelight = 'amber' | 'ruby' | 'sodium' | 'cyan' | 'iris' | 'custom';

export interface Appearance {
  theme: ThemeChoice;
  darkStock: DarkStock;
  lightStock: LightStock;
  accent: Safelight;
  customAccent: string; // '#rrggbb'
  uiScale: 0.9 | 1 | 1.1 | 1.2;
  chatFontSize: number;   // 13..19 int
  chatLineHeight: number; // 1.45..1.9 step .05
  chatFont: 'sans' | 'serif' | 'mono';
  codeFont: 'cascadia-code' | 'cascadia-mono' | 'consolas' | 'jetbrains';
  codeLigatures: boolean;
  density: 'compact' | 'comfortable' | 'spacious';
  chatWidth: 'narrow' | 'standard' | 'wide' | 'full';
  userMessage: 'bubble' | 'plain';
  showStats: boolean;
  showChatTimes: boolean;
  motion: 'system' | 'reduced' | 'full';
  radius: 'sharp' | 'default' | 'round';
  contrast: 'standard' | 'high';
}

export const DEFAULT_APPEARANCE: Appearance = {
  theme: 'dark',
  darkStock: 'umber',
  lightStock: 'fiber',
  accent: 'amber',
  customAccent: '#f08a3c',
  uiScale: 1,
  chatFontSize: 15,
  chatLineHeight: 1.7,
  chatFont: 'sans',
  codeFont: 'cascadia-code',
  codeLigatures: true,
  density: 'comfortable',
  chatWidth: 'standard',
  userMessage: 'bubble',
  showStats: true,
  showChatTimes: true,
  motion: 'system',
  radius: 'default',
  contrast: 'standard',
};

const STORAGE_KEY = 'locallm-appearance';
const LEGACY_THEME_KEY = 'locallm-theme';

const DARK_STOCKS: DarkStock[] = ['umber', 'graphite', 'ink', 'pitch'];
const LIGHT_STOCKS: LightStock[] = ['fiber', 'glossy', 'cool'];
const SAFELIGHTS: Safelight[] = ['amber', 'ruby', 'sodium', 'cyan', 'iris', 'custom'];
const THEMES: ThemeChoice[] = ['dark', 'light', 'system'];
const UI_SCALES: Appearance['uiScale'][] = [0.9, 1, 1.1, 1.2];
const CHAT_FONTS: Appearance['chatFont'][] = ['sans', 'serif', 'mono'];
const CODE_FONTS: Appearance['codeFont'][] = ['cascadia-code', 'cascadia-mono', 'consolas', 'jetbrains'];
const DENSITIES: Appearance['density'][] = ['compact', 'comfortable', 'spacious'];
const CHAT_WIDTHS: Appearance['chatWidth'][] = ['narrow', 'standard', 'wide', 'full'];
const USER_MESSAGES: Appearance['userMessage'][] = ['bubble', 'plain'];
const MOTIONS: Appearance['motion'][] = ['system', 'reduced', 'full'];
const RADII: Appearance['radius'][] = ['sharp', 'default', 'round'];
const CONTRASTS: Appearance['contrast'][] = ['standard', 'high'];

function pick<T>(value: unknown, allowed: readonly T[], fallback: T): T {
  return allowed.includes(value as T) ? (value as T) : fallback;
}

function bool(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback;
}

function clampFontSize(value: unknown, fallback: number): number {
  const n = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(19, Math.max(13, Math.round(n)));
}

function clampLineHeight(value: unknown, fallback: number): number {
  const n = typeof value === 'number' ? value : Number(value);
  if (!Number.isFinite(n)) return fallback;
  const stepped = 1.45 + Math.round((Math.min(1.9, Math.max(1.45, n)) - 1.45) / 0.05) * 0.05;
  return Number(stepped.toFixed(2));
}

export function loadAppearance(): Appearance {
  let raw: Record<string, unknown> = {};
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      const parsed: unknown = JSON.parse(saved);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        raw = parsed as Record<string, unknown>;
      }
    } else {
      // Migrate the pre-appearance theme key once.
      const legacy = localStorage.getItem(LEGACY_THEME_KEY);
      if (legacy === 'light' || legacy === 'system' || legacy === 'dark') raw = { theme: legacy };
    }
  } catch {
    raw = {};
  }
  const d = DEFAULT_APPEARANCE;
  return {
    theme: pick(raw.theme, THEMES, d.theme),
    darkStock: pick(raw.darkStock, DARK_STOCKS, d.darkStock),
    lightStock: pick(raw.lightStock, LIGHT_STOCKS, d.lightStock),
    accent: pick(raw.accent, SAFELIGHTS, d.accent),
    customAccent: typeof raw.customAccent === 'string' && /^#[0-9a-f]{6}$/i.test(raw.customAccent) ? raw.customAccent : d.customAccent,
    uiScale: pick(raw.uiScale, UI_SCALES, d.uiScale),
    chatFontSize: clampFontSize(raw.chatFontSize, d.chatFontSize),
    chatLineHeight: clampLineHeight(raw.chatLineHeight, d.chatLineHeight),
    chatFont: pick(raw.chatFont, CHAT_FONTS, d.chatFont),
    codeFont: pick(raw.codeFont, CODE_FONTS, d.codeFont),
    codeLigatures: bool(raw.codeLigatures, d.codeLigatures),
    density: pick(raw.density, DENSITIES, d.density),
    chatWidth: pick(raw.chatWidth, CHAT_WIDTHS, d.chatWidth),
    userMessage: pick(raw.userMessage, USER_MESSAGES, d.userMessage),
    showStats: bool(raw.showStats, d.showStats),
    showChatTimes: bool(raw.showChatTimes, d.showChatTimes),
    motion: pick(raw.motion, MOTIONS, d.motion),
    radius: pick(raw.radius, RADII, d.radius),
    contrast: pick(raw.contrast, CONTRASTS, d.contrast),
  };
}

export function saveAppearance(a: Appearance): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(a));
    // Keep the legacy key in sync for older builds and simple theme reads.
    localStorage.setItem(LEGACY_THEME_KEY, a.theme);
  } catch {
    /* Private mode or a full quota must not break the UI. */
  }
}

const READING_SERIF = `'Sitka Text', Charter, 'Iowan Old Style', Georgia, serif`;
const MONO_FALLBACK = `'Cascadia Code', 'JetBrains Mono', Consolas, 'Liberation Mono', monospace`;
const CODE_FACES: Record<Appearance['codeFont'], string> = {
  'cascadia-code': `'Cascadia Code'`,
  'cascadia-mono': `'Cascadia Mono'`,
  consolas: 'Consolas',
  jetbrains: `'JetBrains Mono'`,
};

const CHAT_WIDTH_VARS: Record<Appearance['chatWidth'], string> = {
  narrow: '640px', standard: '760px', wide: '920px', full: '100%',
};
const RADIUS_SCALES: Record<Appearance['radius'], string> = { sharp: '0.4', default: '1', round: '1.5' };

export function applyAppearance(a: Appearance, resolved: 'dark' | 'light', root: HTMLElement = document.documentElement): void {
  const data = root.dataset;
  data.theme = resolved;
  data.stock = resolved === 'dark' ? a.darkStock : a.lightStock;
  data.accent = a.accent;
  data.density = a.density;
  data.userMessage = a.userMessage;
  data.motion = a.motion;
  data.contrast = a.contrast;
  if (a.showStats) delete data.hideStats; else data.hideStats = '';
  if (a.showChatTimes) delete data.hideChatTimes; else data.hideChatTimes = '';

  const style = root.style;
  style.setProperty('--ui-scale', String(a.uiScale));
  style.setProperty('--chat-font-size', `${a.chatFontSize}px`);
  style.setProperty('--chat-line-height', String(a.chatLineHeight));
  style.setProperty('--chat-width', CHAT_WIDTH_VARS[a.chatWidth]);
  style.setProperty('--radius-scale', RADIUS_SCALES[a.radius]);
  style.setProperty('--font-chat',
    a.chatFont === 'serif' ? READING_SERIF : a.chatFont === 'mono' ? 'var(--font-code)' : 'var(--font-sans)');
  style.setProperty('--font-code', `${CODE_FACES[a.codeFont]}, ${MONO_FALLBACK}`);
  style.setProperty('--code-ligatures', a.codeLigatures ? 'normal' : 'none');
  style.setProperty('--accent-custom', a.customAccent);
}
