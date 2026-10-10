import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import {
  Activity, ArrowLeft, BookOpen, Box, Cpu, Database, FileText, FolderOpen, Globe, Keyboard,
  Monitor, Moon, MousePointerClick, Palette, Pipette, Plug, RotateCcw, Search, SendHorizontal, SlidersHorizontal, Sparkles, Sun, Terminal, Wrench,
  type LucideIcon,
} from 'lucide-react';
import { DEFAULT_APPEARANCE } from '../lib/appearance';
import type { Appearance, DarkStock, LightStock, Safelight } from '../lib/appearance';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, BrowserConfig, BrowserMode, Conversation, WorkspaceIndex } from '../lib/types';
import type { SettingsSection } from './Sidebar';
import { PERMISSION_OPTIONS } from './ToolControls';
import { moveWithinGroup } from '../lib/roving';

interface RailItem { id: SettingsSection; label: string; icon: LucideIcon; keywords: string }

const GROUPS: { label: string; items: RailItem[] }[] = [
  { label: 'App', items: [
    { id: 'general', label: 'General', icon: SlidersHorizontal, keywords: 'permissions access approval full default project folder new chat' },
    { id: 'appearance', label: 'Appearance', icon: Palette, keywords: 'theme dark light system color accent safelight font size density width motion contrast corners stock' },
    { id: 'shortcuts', label: 'Keyboard shortcuts', icon: Keyboard, keywords: 'keys hotkeys ctrl' },
  ] },
  { label: 'Models', items: [
    { id: 'models', label: 'Models', icon: Box, keywords: 'library discover hugging face download gguf load' },
    { id: 'runtime', label: 'Runtime', icon: Cpu, keywords: 'gpu cpu context layers threads llama.cpp memory vram' },
    { id: 'generation', label: 'Generation', icon: Sparkles, keywords: 'temperature top-p tokens system prompt instructions' },
    { id: 'model-files', label: 'Model files', icon: FolderOpen, keywords: 'llama-server executable projector path browse' },
    { id: 'diagnostics', label: 'Diagnostics', icon: Activity, keywords: 'logs runtime errors' },
  ] },
  { label: 'Agent', items: [
    { id: 'tools', label: 'Tools', icon: Wrench, keywords: 'workspace files code execution preset profile' },
    { id: 'connectors', label: 'Connectors', icon: Plug, keywords: 'mcp services oauth api token' },
    { id: 'skills', label: 'Skills', icon: BookOpen, keywords: 'instructions install activate' },
    { id: 'execution', label: 'Execution', icon: Terminal, keywords: 'python node powershell interpreter daytona cloud' },
    { id: 'web-search', label: 'Web search', icon: Globe, keywords: 'searxng google search provider fallback' },
    { id: 'browser', label: 'Browser', icon: MousePointerClick, keywords: 'browser use agent click website automation pages actions limit' },
    { id: 'documents', label: 'Documents', icon: FileText, keywords: 'local files collections private knowledge search index citation folder' },
  ] },
  { label: 'Data', items: [
    { id: 'data', label: 'Data & archive', icon: Database, keywords: 'export archived chats projects storage' },
  ] },
];

export function SettingsShell({ section, onSection, onClose, children }: {
  section: SettingsSection; onSection: (section: SettingsSection) => void; onClose: () => void; children: ReactNode;
}) {
  const [query, setQuery] = useState('');
  const content = useRef<HTMLDivElement>(null);
  const q = query.trim().toLowerCase();
  const groups = GROUPS.map(group => ({ ...group, items: group.items.filter(item => !q || `${item.label} ${item.keywords}`.toLowerCase().includes(q)) }))
    .filter(group => group.items.length);
  useEffect(() => { if (content.current) content.current.scrollTop = 0; }, [section]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape' && !document.querySelector('dialog[open]')) onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);
  return <div className="settings-shell">
    <nav className="settings-rail" aria-label="Settings sections">
      <button type="button" className="settings-back" onClick={onClose}><ArrowLeft size={15} />Back to app</button>
      <label className="settings-search">
        <Search size={14} aria-hidden="true" />
        <input type="search" aria-label="Search settings" placeholder="Search settings" value={query} onChange={event => setQuery(event.target.value)}
          onKeyDown={event => { if (event.key === 'Enter' && groups[0]) onSection(groups[0].items[0].id); }} />
      </label>
      <div className="settings-rail-scroll">
        {groups.map(group => <div className="settings-rail-group" key={group.label}>
          <p className="settings-rail-label">{group.label}</p>
          {group.items.map(({ id, label, icon: Icon }) => <button type="button" key={id} className="settings-rail-item"
            aria-current={section === id ? 'page' : undefined} onClick={() => onSection(id)}>
            <Icon size={15} aria-hidden="true" /><span>{label}</span>
          </button>)}
        </div>)}
        {!groups.length && <p className="settings-rail-empty">No settings match “{query}”.</p>}
      </div>
    </nav>
    <div className="settings-content" ref={content}>{children}</div>
  </div>;
}

export function SettingsHeader({ title, description, children }: { title: string; description?: ReactNode; children?: ReactNode }) {
  return <header className="settings-header">
    <div><h1>{title}</h1>{description && <p>{description}</p>}</div>
    {children && <div className="settings-header-actions">{children}</div>}
  </header>;
}

export function SettingsGroup({ title, description, children, className = '' }: { title?: string; description?: ReactNode; children: ReactNode; className?: string }) {
  return <section className={`settings-group ${className}`}>
    {title && <h2>{title}</h2>}
    {description && <p className="settings-group-description">{description}</p>}
    <div className="settings-card">{children}</div>
  </section>;
}

export function SettingRow({ label, description, children, htmlFor, stacked = false }: { label: ReactNode; description?: ReactNode; children?: ReactNode; htmlFor?: string; stacked?: boolean }) {
  return <div className={`setting-row${stacked ? ' stacked' : ''}`}>
    <div className="setting-row-copy">
      {htmlFor ? <label htmlFor={htmlFor}>{label}</label> : <strong>{label}</strong>}
      {description && <p>{description}</p>}
    </div>
    {children && <div className="setting-row-control">{children}</div>}
  </div>;
}

export interface RadioCardOption<T extends string> { value: T; label: string; description?: string; badge?: string; tone?: 'default' | 'info' | 'warning' }

export function RadioCards<T extends string>({ name, label, value, options, onChange, disabled }: {
  name: string; label: string; value: T; options: RadioCardOption<T>[]; onChange: (value: T) => void; disabled?: boolean;
}) {
  return <div className="radio-cards" role="radiogroup" aria-label={label}>
    {options.map(option => <label key={option.value} className={`radio-card${value === option.value ? ' selected' : ''}${option.tone ? ` tone-${option.tone}` : ''}`}>
      <input type="radio" name={name} value={option.value} checked={value === option.value} disabled={disabled} onChange={() => onChange(option.value)} />
      <span className="radio-card-copy">
        <span className="radio-card-title">{option.label}{option.badge && <span className="radio-card-badge">{option.badge}</span>}</span>
        {option.description && <span className="radio-card-description">{option.description}</span>}
      </span>
    </label>)}
  </div>;
}

export type { ThemeChoice } from '../lib/appearance';

export function GeneralSettings({ accessMode, onAccessModeChange, busy, nativeAvailable }: {
  accessMode: AccessMode; onAccessModeChange: (mode: AccessMode) => void; busy: boolean; nativeAvailable: boolean;
}) {
  return <div className="settings-page">
    <SettingsHeader title="General" description="How the assistant works by default." />
    <SettingsGroup title="Permissions" description="Applies to the open chat and becomes the default for new chats.">
      <RadioCards name="access-mode" label="Default permissions" value={accessMode} disabled={busy || !nativeAvailable} onChange={onAccessModeChange}
        options={PERMISSION_OPTIONS.map(option => ({
          value: option.key, label: option.label, description: option.desc,
          badge: option.key === 'ask' ? 'Default' : option.badge,
          tone: option.key === 'fullAccess' ? 'warning' : option.key === 'autoApprove' ? 'info' : 'default',
        }))} />
    </SettingsGroup>
    <p className="settings-note">Each chat keeps its own tools and permission choice. Open Tools to choose what the assistant can use.</p>
  </div>;
}

export function Segmented<T extends string>({ label, value, options, onChange }: {
  label: string; value: T; options: { value: T; label: string; icon?: LucideIcon }[]; onChange: (value: T) => void;
}) {
  return <div className="segmented" role="radiogroup" aria-label={label}
    onKeyDown={event => moveWithinGroup(event, options.map(option => option.value), value, onChange)}>
    {options.map(({ value: option, label: optionLabel, icon: Icon }) => <button type="button" key={option} role="radio" aria-checked={value === option}
      tabIndex={value === option ? 0 : -1}
      className={value === option ? 'selected' : ''} onClick={() => onChange(option)}>{Icon && <Icon size={14} aria-hidden="true" />}{optionLabel}</button>)}
  </div>;
}

function StockChip<T extends string>({ stock, name, group, selected, onSelect }: {
  stock: T; name: string; group: string; selected: boolean; onSelect: (stock: T) => void;
}) {
  return <label className={`stock-chip${selected ? ' selected' : ''}`}>
    <input type="radio" className="sr-only" name={group} value={stock} checked={selected} onChange={() => onSelect(stock)} />
    <span className="stock-chip-preview" data-stock={stock} aria-hidden="true">
      <span className="stock-chip-side" />
      <span className="stock-chip-body"><span className="stock-chip-card" /><span className="stock-chip-line" /></span>
    </span>
    <span className="stock-chip-name">{name}</span>
  </label>;
}

const DARK_STOCKS: { value: DarkStock; label: string }[] = [
  { value: 'umber', label: 'Umber' }, { value: 'graphite', label: 'Graphite' },
  { value: 'ink', label: 'Ink' }, { value: 'pitch', label: 'Pitch' },
];
const LIGHT_STOCKS: { value: LightStock; label: string }[] = [
  { value: 'fiber', label: 'Fiber' }, { value: 'glossy', label: 'Glossy' }, { value: 'cool', label: 'Cool' },
];
const SAFELIGHTS: { value: Exclude<Safelight, 'custom'>; label: string }[] = [
  { value: 'amber', label: 'Amber' }, { value: 'ruby', label: 'Ruby' }, { value: 'sodium', label: 'Sodium' },
  { value: 'cyan', label: 'Cyan' }, { value: 'iris', label: 'Iris' },
];

/* Pick an icon color that reads on top of the chosen custom accent. */
function swatchIconColor(hex: string): string {
  const value = Number.parseInt(hex.slice(1), 16);
  if (!Number.isFinite(value)) return 'rgba(0, 0, 0, .7)';
  const luminance = (0.2126 * ((value >> 16) & 255) + 0.7152 * ((value >> 8) & 255) + 0.0722 * (value & 255)) / 255;
  return luminance > 0.55 ? 'rgba(0, 0, 0, .7)' : '#fff';
}

/* A miniature of the workspace, built purely from tokens: every control
   above mutates a CSS variable or data-* attribute, so it reacts live. */
function AppearanceProof({ userMessage }: { userMessage: Appearance['userMessage'] }) {
  return <div className="appearance-proof-wrap">
    <div className="appearance-proof" data-user-message={userMessage} aria-hidden="true">
      <div className="proof-sidebar">
        <span className="proof-brand" />
        <span className="proof-row" />
        <span className="proof-row proof-active" />
        <span className="proof-row" />
      </div>
      <div className="proof-main">
        <div className="proof-transcript">
          <div className="proof-turn">
            <div className="proof-user">Give the darkroom a quieter hierarchy.</div>
          </div>
          <p className="proof-work">Worked for <span className="proof-duration">1m 39s</span> · 6 steps</p>
          <p className="proof-answer">The surface stack stays warm and flat, the safelight only ever marks where you are, and <code>run.status</code> keeps its mono numerals.</p>
        </div>
        <div className="proof-composer">
          <span className="proof-placeholder">Do anything</span>
          <span className="proof-send"><SendHorizontal size={11} /></span>
        </div>
      </div>
    </div>
  </div>;
}

export function AppearanceSettings({ appearance, onChange }: { appearance: Appearance; onChange: (next: Appearance) => void }) {
  const isDefault = (Object.keys(DEFAULT_APPEARANCE) as (keyof Appearance)[]).every(key => appearance[key] === DEFAULT_APPEARANCE[key]);
  const set = <K extends keyof Appearance>(key: K, value: Appearance[K]) => onChange({ ...appearance, [key]: value });
  return <div className="settings-page appearance-page">
    <SettingsHeader title="Appearance" description="Tune how LocalLM looks and reads on this computer.">
      <button type="button" className="secondary" disabled={isDefault} onClick={() => onChange(DEFAULT_APPEARANCE)}><RotateCcw size={13} aria-hidden="true" />Reset to defaults</button>
    </SettingsHeader>
    <AppearanceProof userMessage={appearance.userMessage} />

    <SettingsGroup title="Theme">
      <SettingRow label="Mode" description="System follows your Windows light or dark setting.">
        <Segmented label="Theme" value={appearance.theme} onChange={value => set('theme', value)} options={[
          { value: 'dark', label: 'Dark', icon: Moon }, { value: 'light', label: 'Light', icon: Sun }, { value: 'system', label: 'System', icon: Monitor },
        ]} />
      </SettingRow>
      <SettingRow label="Dark stock" description="The neutral palette dark mode is printed on.">
        <div className="stock-chips" role="radiogroup" aria-label="Dark stock">
          {DARK_STOCKS.map(stock => <StockChip key={stock.value} stock={stock.value} name={stock.label} group="dark-stock"
            selected={appearance.darkStock === stock.value} onSelect={value => set('darkStock', value)} />)}
        </div>
      </SettingRow>
      <SettingRow label="Light stock" description="The paper light mode is printed on.">
        <div className="stock-chips" role="radiogroup" aria-label="Light stock">
          {LIGHT_STOCKS.map(stock => <StockChip key={stock.value} stock={stock.value} name={stock.label} group="light-stock"
            selected={appearance.lightStock === stock.value} onSelect={value => set('lightStock', value)} />)}
        </div>
      </SettingRow>
    </SettingsGroup>

    <SettingsGroup title="Safelight" description="Marks where you are and what is running. Nothing else.">
      <SettingRow label="Accent" description="Pick a preset, or choose any color with the custom dot.">
        <div className="swatch-row" role="radiogroup" aria-label="Safelight">
          {SAFELIGHTS.map(light => <label key={light.value} className={`swatch${appearance.accent === light.value ? ' selected' : ''}`}
            data-accent={light.value} title={light.label}>
            <input type="radio" className="sr-only" name="safelight" value={light.value} aria-label={light.label}
              checked={appearance.accent === light.value} onChange={() => set('accent', light.value)} />
            <span className="swatch-dot" />
          </label>)}
          <label className={`swatch swatch-custom${appearance.accent === 'custom' ? ' selected' : ''}`} data-accent="custom"
            title="Custom color" style={{ '--accent-custom': appearance.customAccent } as CSSProperties}>
            <input type="color" value={appearance.customAccent} aria-label="Custom color"
              onClick={() => set('accent', 'custom')}
              onChange={event => onChange({ ...appearance, accent: 'custom', customAccent: event.target.value })} />
            <span className="swatch-dot" />
            <Pipette size={13} className="swatch-custom-icon" style={{ color: swatchIconColor(appearance.customAccent) }} aria-hidden="true" />
          </label>
        </div>
      </SettingRow>
    </SettingsGroup>

    <SettingsGroup title="Text">
      <SettingRow label="Interface size" description="Scales menus, labels and controls; the transcript size is set separately.">
        <Segmented label="Interface size" value={String(appearance.uiScale) as '0.9' | '1' | '1.1' | '1.2'} onChange={value => set('uiScale', Number(value) as Appearance['uiScale'])} options={[
          { value: '0.9', label: 'Small' }, { value: '1', label: 'Default' }, { value: '1.1', label: 'Large' }, { value: '1.2', label: 'Larger' },
        ]} />
      </SettingRow>
      <SettingRow label="Chat text size" htmlFor="chat-font-size" description="Body text in the transcript and the composer.">
        <input id="chat-font-size" type="range" min={13} max={19} step={1} value={appearance.chatFontSize}
          onChange={event => set('chatFontSize', Number(event.target.value))} />
        <span className="range-value">{appearance.chatFontSize}px</span>
      </SettingRow>
      <SettingRow label="Chat line height" htmlFor="chat-line-height">
        <input id="chat-line-height" type="range" min={1.45} max={1.9} step={0.05} value={appearance.chatLineHeight}
          onChange={event => set('chatLineHeight', Number(event.target.value))} />
        <span className="range-value">{appearance.chatLineHeight.toFixed(2)}</span>
      </SettingRow>
      <SettingRow label="Chat font" description="The face used for messages and the composer.">
        <Segmented label="Chat font" value={appearance.chatFont} onChange={value => set('chatFont', value)} options={[
          { value: 'sans', label: 'Sans' }, { value: 'serif', label: 'Reading serif' }, { value: 'mono', label: 'Mono' },
        ]} />
      </SettingRow>
      <SettingRow label="Code font" htmlFor="code-font" description="Used in code blocks, terminals and file views.">
        <select id="code-font" value={appearance.codeFont} onChange={event => set('codeFont', event.target.value as Appearance['codeFont'])}>
          <option value="cascadia-code">Cascadia Code</option>
          <option value="cascadia-mono">Cascadia Mono</option>
          <option value="consolas">Consolas</option>
          <option value="jetbrains">JetBrains Mono</option>
        </select>
      </SettingRow>
      <SettingRow label="Code ligatures" htmlFor="code-ligatures" description="Joins shapes like =&gt; in fonts that support it.">
        <input id="code-ligatures" type="checkbox" className="switch" checked={appearance.codeLigatures}
          onChange={event => set('codeLigatures', event.target.checked)} />
      </SettingRow>
    </SettingsGroup>

    <SettingsGroup title="Layout">
      <SettingRow label="Density" description="Row heights and the gap between turns.">
        <Segmented label="Density" value={appearance.density} onChange={value => set('density', value)} options={[
          { value: 'compact', label: 'Compact' }, { value: 'comfortable', label: 'Comfortable' }, { value: 'spacious', label: 'Spacious' },
        ]} />
      </SettingRow>
      <SettingRow label="Chat width" description="How wide the transcript and composer can grow.">
        <Segmented label="Chat width" value={appearance.chatWidth} onChange={value => set('chatWidth', value)} options={[
          { value: 'narrow', label: 'Narrow' }, { value: 'standard', label: 'Standard' }, { value: 'wide', label: 'Wide' }, { value: 'full', label: 'Full' },
        ]} />
      </SettingRow>
      <SettingRow label="Your messages" description="Bubbles sit right in a soft fill; plain sits flush left under a divider.">
        <Segmented label="Your messages" value={appearance.userMessage} onChange={value => set('userMessage', value)} options={[
          { value: 'bubble', label: 'Bubble' }, { value: 'plain', label: 'Plain' },
        ]} />
      </SettingRow>
      <SettingRow label="Show speed stats" htmlFor="show-stats" description="PPS, TTFT and TGS under each answer.">
        <input id="show-stats" type="checkbox" className="switch" checked={appearance.showStats}
          onChange={event => set('showStats', event.target.checked)} />
      </SettingRow>
      <SettingRow label="Show chat times" htmlFor="show-chat-times" description="Relative times in the sidebar.">
        <input id="show-chat-times" type="checkbox" className="switch" checked={appearance.showChatTimes}
          onChange={event => set('showChatTimes', event.target.checked)} />
      </SettingRow>
    </SettingsGroup>

    <SettingsGroup title="Motion & shape">
      <SettingRow label="Motion" description="System follows your Windows animation setting.">
        <Segmented label="Motion" value={appearance.motion} onChange={value => set('motion', value)} options={[
          { value: 'system', label: 'System' }, { value: 'reduced', label: 'Reduced' }, { value: 'full', label: 'Full' },
        ]} />
      </SettingRow>
      <SettingRow label="Corners">
        <Segmented label="Corners" value={appearance.radius} onChange={value => set('radius', value)} options={[
          { value: 'sharp', label: 'Sharp' }, { value: 'default', label: 'Default' }, { value: 'round', label: 'Round' },
        ]} />
      </SettingRow>
      <SettingRow label="Contrast" description="High darkens muted text and strengthens hairlines.">
        <Segmented label="Contrast" value={appearance.contrast} onChange={value => set('contrast', value)} options={[
          { value: 'standard', label: 'Standard' }, { value: 'high', label: 'High' },
        ]} />
      </SettingRow>
    </SettingsGroup>
  </div>;
}

const BROWSER_MODES: RadioCardOption<BrowserMode>[] = [
  { value: 'off', label: 'Off', description: 'The assistant never opens the browser. Answers come from its knowledge, web search, and page reads.' },
  { value: 'auto', label: 'Automatic', badge: 'Default', description: 'Used only when a task needs a live page: a site you name, logging in, filling forms, or testing a local app.' },
  { value: 'preferred', label: 'Prefer the browser', tone: 'info', description: 'Opens pages whenever an answer depends on a live website or current information, instead of answering from memory. Slower, but checked.' },
];
const MAX_BROWSER_ACTIONS = 50;

export function BrowserSettings() {
  const [config, setConfig] = useState<BrowserConfig | null>(null);
  const [limit, setLimit] = useState('');
  const [status, setStatus] = useState('');
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  // Last value the backend confirmed; a failed save reverts the display to it.
  const confirmed = useRef<BrowserConfig | null>(null);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const request = useRef(0);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getBrowserConfig()
      .then(value => { if (!disposed) { confirmed.current = value; setConfig(value); setLimit(String(value.maxActions)); } })
      .catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);

  function save(next: BrowserConfig) {
    setConfig(next); setLimit(String(next.maxActions)); setError(''); setStatus('Saving…'); setSaving(true);
    const seq = ++request.current;
    queue.current = queue.current
      .catch(() => {})
      .then(() => api.saveBrowserConfig(next))
      .then(() => {
        confirmed.current = next;
        if (seq === request.current) {
          setStatus('Saved. Applies from your next message.');
          setSaving(false);
        }
      })
      .catch(e => {
        if (seq === request.current) {
          setConfig(confirmed.current);
          setLimit(confirmed.current ? String(confirmed.current.maxActions) : '');
          setStatus('');
          setError(`Not saved: ${errorMessage(e)}. Showing your saved setting.`);
          setSaving(false);
        }
      });
  }

  function commitLimit() {
    if (!config) return;
    const parsed = Math.round(Number(limit));
    const maxActions = Number.isFinite(parsed) ? Math.min(MAX_BROWSER_ACTIONS, Math.max(1, parsed)) : config.maxActions;
    if (maxActions === config.maxActions) setLimit(String(maxActions));
    else void save({ ...config, maxActions });
  }

  const disabled = !config;
  return <div className="settings-page">
    <SettingsHeader title="Browser" description="Lets the assistant open and use websites in a browser tab you can watch in the workspace panel." />
    <SettingsGroup title="When to use the browser" description="Applies to every chat. The browser also needs a tool preset that includes it (Standard, Research, or Coding).">
      <RadioCards name="browser-mode" label="Browser use" value={config?.mode ?? 'auto'} options={BROWSER_MODES} disabled={disabled} onChange={mode => config && void save({ ...config, mode })} />
    </SettingsGroup>
    <SettingsGroup title="Limits">
      <SettingRow label="Browser actions per reply" htmlFor="browser-max-actions"
        description={`Each click, page load, or scroll counts as one. When the limit is reached, the assistant answers with what it found. 1–${MAX_BROWSER_ACTIONS}.`}>
        <input id="browser-max-actions" type="number" inputMode="numeric" min={1} max={MAX_BROWSER_ACTIONS} step={1} value={limit}
          disabled={disabled || config.mode === 'off'} onChange={event => setLimit(event.target.value)} onBlur={commitLimit}
          onKeyDown={event => { if (event.key === 'Enter') commitLimit(); }} />
      </SettingRow>
    </SettingsGroup>
    <p className="settings-note">Browser actions follow your permission setting: in Ask mode, each one needs approval.</p>
    {!nativeAvailable && <p className="settings-note">Browser settings need the desktop app.</p>}
    {saving && <p role="status">Saving…</p>}
    {error && <p role="alert" className="error-banner">{error}</p>}
    {status && !saving && <p role="status" className="success">{status}</p>}
  </div>;
}

const SHORTCUTS: [string, string[]][] = [
  ['New chat', ['Ctrl', 'N']],
  ['Search chats', ['Ctrl', 'K']],
  ['Send message', ['Enter']],
  ['New line in message', ['Shift', 'Enter']],
  ['Close settings or menus', ['Esc']],
  ['Review changes', ['Ctrl', 'Shift', 'G']],
  ['Terminal panel', ['Ctrl', '`']],
  ['Browser panel', ['Ctrl', 'T']],
  ['Files panel', ['Ctrl', 'P']],
];

export function ShortcutSettings() {
  return <div className="settings-page">
    <SettingsHeader title="Keyboard shortcuts" />
    <SettingsGroup>
      {SHORTCUTS.map(([label, keys]) => <SettingRow key={label} label={label}>
        <span className="key-combo">{keys.map(key => <kbd key={key}>{key}</kbd>)}</span>
      </SettingRow>)}
    </SettingsGroup>
  </div>;
}

/// Auto-retain is a plain setting; the toggle loads it lazily so this page
/// stays prop-driven.
function MemoryRetainRow() {
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    api.memoryAutoRetain().then(setEnabled).catch(caught => setError(errorMessage(caught)));
  }, []);
  return <SettingRow label="Auto-retain agent memories" description="When off, the agent's memory tools decline to store anything. Existing memories stay listed under Context.">
    <input type="checkbox" checked={enabled ?? true} disabled={enabled === null} aria-label="Auto-retain agent memories"
      onChange={event => {
        const next = event.target.checked;
        setEnabled(next);
        void api.memorySetAutoRetain(next).catch(caught => { setEnabled(!next); setError(errorMessage(caught)); });
      }} />
    {error && <p className="error-banner" role="alert">{error}</p>}
  </SettingRow>;
}

export function DataSettings({ workspace, conversations, activeTitle, canExport, onExport, onRestore, busy }: {
  workspace: WorkspaceIndex; conversations: Conversation[]; activeTitle?: string;
  canExport: boolean; onExport: () => void; onRestore: (id: string) => void; busy: boolean;
}) {
  const archived = conversations.filter(item => workspace.tasks[item.id]?.archived);
  return <div className="settings-page">
    <SettingsHeader title="Data & archive" description="Conversations and settings are stored only on this computer." />
    <SettingsGroup title="Export">
      <SettingRow label="Save a copy of this chat" description={activeTitle ? `Exports “${activeTitle}” as Markdown or complete JSON.` : 'Open a chat first to export it.'}>
        <button type="button" className="secondary" disabled={!canExport || busy} onClick={onExport}>Export…</button>
      </SettingRow>
    </SettingsGroup>
    <SettingsGroup title="Archived chats">
      {archived.length ? archived.map(item => <SettingRow key={item.id} label={item.title} description={new Date(item.updatedAt).toLocaleString()}>
        <button type="button" className="secondary" disabled={busy} aria-label={`Restore ${item.title}`} onClick={() => onRestore(item.id)}>Restore</button>
      </SettingRow>) : <p className="settings-empty">No archived chats. Archive a chat from its menu in the sidebar.</p>}
    </SettingsGroup>
    <SettingsGroup title="Memory">
      <MemoryRetainRow />
    </SettingsGroup>
  </div>;
}
