import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Check, ChevronDown, ImagePlus, Monitor, Moon, Play, RotateCcw, Sun } from 'lucide-react';
import { accentColors, colorGroups, contrastRatio, defaultAppearance, hexColor, palettePresets, paletteTokens, prefersReducedMotion, type PaletteOverrides, type PaletteToken, type AppearancePreferences, type ResolvedTheme, type ThemePreference } from '../lib/appearance';
import { homeWallpapers, prepareWallpaper, wallpaperUrl, type HomeWallpaper } from '../lib/homeWallpaper';
import { CodeBlock } from './CodeBlock';
import './AppearanceSettings.css';

const themes = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
] as const;

function ThemePreview({ theme }: { theme: ThemePreference }) {
  return <span className={`theme-preview theme-preview-${theme}`} aria-hidden="true"><span className="theme-preview-rail"><i /><i /><i /></span><span className="theme-preview-sidebar"><i /><i /><i /></span><span className="theme-preview-canvas"><span /><span /><span className="theme-preview-composer" /></span></span>;
}

const codeSamples = {
  typescript: `// Build a greeting for the workspace.
interface Profile { name: string }

function greet(profile: Profile) {
  const count = 3;
  return "Hello, " + profile.name.repeat(count);
}`,
  python: `# Build a greeting for the workspace.
def greet(name: str) -> str:
    count = 3
    return f"Hello, {name}" * count`,
  xml: `<!-- Theme preview -->
<section class="welcome">
  <a href="/workspace">Open workspace</a>
</section>`,
  markdown: `# Workspace notes

- [Open workspace](/workspace)
- Read the \`README\` before editing.`,
} as const;

function ColorControl({ token, label, value, resolved, onChange }: {
  token: PaletteToken; label: string; value?: string; resolved?: string;
  onChange: (token: PaletteToken, color?: string) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const id = `appearance${token}`;
  const invalid = draft !== null && !hexColor.test(draft.trim());
  const current = value ?? resolved ?? '';
  function commit() {
    if (draft === null) return;
    const text = draft.trim();
    if (hexColor.test(text)) {
      onChange(token, text);
      setDraft(null);
    }
  }

  return <div className="appearance-color">
    <label htmlFor={id}>{label}</label>
    <div className="appearance-color-controls">
      <input type="color" aria-label={`Pick ${label.toLowerCase()} color`} value={hexColor.test(current) ? current : '#000000'} onChange={event => { setDraft(null); onChange(token, event.target.value); }} />
      <input id={id} type="text" value={draft ?? current.toUpperCase()} placeholder="#RRGGBB" maxLength={7} spellCheck={false}
        aria-invalid={invalid} aria-describedby={invalid ? `${id}-error` : undefined}
        onChange={event => setDraft(event.target.value)} onBlur={commit}
        onKeyDown={event => {
          if (event.key === 'Enter') { event.preventDefault(); commit(); }
          if (event.key === 'Escape' && draft !== null) { event.stopPropagation(); setDraft(null); }
        }} />
      <button type="button" className="secondary" aria-label={`Reset ${label.toLowerCase()} color`} title={`Reset ${label.toLowerCase()} color`}
        disabled={!value && draft === null} onClick={() => { setDraft(null); onChange(token); }}><RotateCcw size={13} /></button>
    </div>
    {invalid && <small id={`${id}-error`}>Use # followed by six hex digits.</small>}
  </div>;
}

function PaletteStudio({ preferences, theme, onChange }: {
  preferences: AppearancePreferences; theme: ResolvedTheme; onChange: (patch: Partial<AppearancePreferences>) => void;
}) {
  const [language, setLanguage] = useState<keyof typeof codeSamples>('typescript');
  const [resolved, setResolved] = useState<PaletteOverrides>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const key = theme === 'dark' ? 'darkColors' : 'lightColors';
  const colors = preferences[key];
  const hasOverrides = Object.keys(colors).length > 0;
  const needsResolvedColors = hasOverrides || Object.values(expanded).some(Boolean);

  useEffect(() => {
    if (!needsResolvedColors) return;
    // The parent applies the palette in a layout effect before this read.
    const styles = getComputedStyle(document.documentElement);
    setResolved(Object.fromEntries(paletteTokens.map(token => [token, styles.getPropertyValue(token).trim()])));
  }, [preferences.palette, preferences.accent, theme, colors, needsResolvedColors]);

  const updateColor = (token: PaletteToken, color?: string) => {
    const next = { ...colors };
    if (color) next[token] = color;
    else delete next[token];
    onChange({ [key]: next });
  };

  const allColors = { ...resolved, ...colors };
  const pairs = [
    ['--text', '--background'], ['--text', '--surface'], ['--text', '--input'],
    ['--secondary', '--sidebar'], ['--secondary', '--surface'],
    ['--code-fg', '--code-bg'], ['--code-gutter', '--code-bg'],
    ...paletteTokens.filter(token => token.startsWith('--syn-')).map(token => [token, '--code-bg'] as const),
  ] as const;
  const warnings = new Set<string>();
  const labels: Record<string, string> = Object.assign({}, ...colorGroups.map(group => group.colors));
  for (const [fg, bg] of pairs) {
    const foreground = allColors[fg];
    const background = allColors[bg];
    if ((colors[fg] || colors[bg]) && foreground && background && hexColor.test(foreground) && hexColor.test(background) && contrastRatio(foreground, background) < 4.5) {
      warnings.add(`${labels[fg]} on ${labels[bg].toLowerCase()}`);
    }
  }

  return <section className="preference-section" aria-labelledby="appearance-palette-heading">
    <div className="preference-section-heading">
      <h2 id="appearance-palette-heading">Palette studio</h2>
      <p>Build a custom theme from a preset. Light and dark keep separate overrides, even when System switches between them. Pick a color or enter a hex value and press Enter.</p>
    </div>
    <div className="preference-group">
      <div className="preference-row">
        <div className="preference-copy"><label htmlFor="appearance-palette">Base palette</label><p>Custom colors stay in place when you change the base.</p></div>
        <select id="appearance-palette" value={preferences.palette} onChange={event => onChange({ palette: event.target.value as AppearancePreferences['palette'] })}>
          {Object.entries(palettePresets).map(([value, label]) => <option value={value} key={value}>{label}</option>)}
        </select>
      </div>
    </div>
    <div className="appearance-studio">
      <div className="appearance-studio-preview" role="region" aria-label="Live theme preview">
        <div className="appearance-studio-title"><strong>Live preview</strong><span className="settings-device-label">{theme === 'dark' ? 'Dark' : 'Light'}</span></div>
        <div className="appearance-chat-sample">
          <div className="appearance-chat-nav"><span aria-hidden="true">●</span> Workspace</div>
          <div className="appearance-chat-message"><span>Assistant</span><p>Here’s a small example for your project.</p></div>
          <div className="appearance-chat-input">Write a message… <b>Send</b></div>
        </div>
        <div className="appearance-studio-title"><label htmlFor="appearance-sample-language">Code preview</label>
          <select id="appearance-sample-language" value={language} onChange={event => setLanguage(event.target.value as keyof typeof codeSamples)}>
            <option value="typescript">TypeScript</option><option value="python">Python</option><option value="xml">HTML</option><option value="markdown">Markdown</option>
          </select>
        </div>
        <CodeBlock code={codeSamples[language]} language={language} ariaLabel="Syntax color preview" lineNumbers />
        <div className="appearance-syntax-legend" aria-label="Syntax color swatches">
          {Object.entries(colorGroups[2].colors).map(([token, label]) => <span key={token}><i style={{ backgroundColor: `var(${token})` }} />{label}</span>)}
        </div>
      </div>
      <div className="appearance-palette-controls" key={theme}>
        <div className="appearance-studio-title"><strong>Editing {theme} colors</strong><button type="button" className="secondary" disabled={!hasOverrides}
          onClick={() => onChange({ [key]: {} })}>Reset {theme} colors</button></div>
        {warnings.size > 0 && <p className="appearance-contrast" role="status">Low contrast (below 4.5:1): {[...warnings].join('; ')}.</p>}
        {colorGroups.map((group, index) => {
          const open = !!expanded[group.label];
          const panelId = `appearance-color-group-${index}`;
          return <div key={group.label} className="appearance-color-group">
            <button type="button" aria-expanded={open} aria-controls={panelId}
              onClick={() => setExpanded(current => ({ ...current, [group.label]: !current[group.label] }))}>
              <span aria-hidden="true">{open ? '▾' : '▸'}</span> {group.label}
            </button>
            <div id={panelId} hidden={!open}>{open && <div className="appearance-color-grid">{Object.entries(group.colors).map(([name, label]) => {
              const token = name as PaletteToken;
              return <ColorControl key={token} token={token} label={label} value={colors[token]} resolved={resolved[token]} onChange={updateColor} />;
            })}</div>}</div>
          </div>;
        })}
      </div>
    </div>
  </section>;
}

function MotionPreview({ motion, speed }: Pick<AppearancePreferences, 'motion'> & { speed: AppearancePreferences['motionSpeed'] }) {
  const [systemReduced, setSystemReduced] = useState(() => typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const [run, setRun] = useState(0);
  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(prefers-reduced-motion: reduce)');
    const sync = () => setSystemReduced(media.matches);
    sync();
    media.addEventListener('change', sync);
    return () => media.removeEventListener('change', sync);
  }, []);
  const reduced = motion === 'reduced' || systemReduced;

  return <div className="appearance-motion-preview">
    <div className="appearance-studio-title"><strong>Motion preview</strong>
      <button type="button" className="secondary" disabled={reduced} onClick={() => { if (!prefersReducedMotion()) setRun(value => value + 1); }}><Play size={13} />Replay animation</button>
    </div>
    <div className="appearance-motion-track">
      <div key={`${run}-${speed}-${reduced}`} className={`appearance-motion-card${reduced ? '' : ' is-animated'}`}>
        <span aria-hidden="true">✓</span><div><strong>Workspace ready</strong><p>Preview of a panel opening.</p></div>
      </div>
    </div>
    <p className="appearance-preview-caption" role="status">{reduced ? 'The preview stays still while reduced motion is active.' : 'Change the animation speed or replay the panel entrance.'}</p>
  </div>;
}

function HomeWallpaperSettings({ preferences, onChange }: {
  preferences: AppearancePreferences; onChange: (patch: Partial<AppearancePreferences>) => void;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const applyImage = useRef<typeof onChange | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  useLayoutEffect(() => {
    applyImage.current = onChange;
    return () => { applyImage.current = null; };
  }, [onChange]);

  async function upload(file: File) {
    setLoading(true);
    setError('');
    try {
      const image = await prepareWallpaper(file);
      applyImage.current?.({ homeWallpaper: 'custom', customWallpaper: image });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Could not set this wallpaper.');
    } finally {
      setLoading(false);
    }
  }

  const choices: [HomeWallpaper, string][] = [
    ['none', 'None'],
    ...Object.entries(homeWallpapers) as [keyof typeof homeWallpapers, string][],
  ];
  if (preferences.customWallpaper) choices.push(['custom', 'My image']);

  return <section className="preference-section" aria-labelledby="appearance-wallpaper-heading">
    <div className="preference-section-heading">
      <h2 id="appearance-wallpaper-heading">Chat wallpaper</h2>
      <p>Choose a background for the chat screen and your conversations.</p>
    </div>
    <div className="preference-group wallpaper-settings">
      <div className="wallpaper-options" role="group" aria-label="Chat wallpaper">
        {choices.map(([id, label]) => {
          const url = wallpaperUrl(id, preferences.customWallpaper);
          const selected = preferences.homeWallpaper === id;
          return <button key={id} type="button" className="wallpaper-option" aria-pressed={selected} disabled={loading}
            onClick={() => { setError(''); onChange({ homeWallpaper: id }); }}>
            <span className="wallpaper-thumbnail" aria-hidden="true">
              {url && <img src={url} alt="" loading="lazy" />}
              {selected && <span className="wallpaper-check"><Check size={13} /></span>}
            </span>
            <span>{label}</span>
          </button>;
        })}
      </div>
      <div className="wallpaper-actions">
        <input ref={fileInput} type="file" hidden accept="image/jpeg,image/png,image/webp"
          onChange={event => {
            const file = event.target.files?.[0];
            event.target.value = '';
            if (file) void upload(file);
          }} />
        <button type="button" className="secondary" disabled={loading} onClick={() => fileInput.current?.click()}>
          <ImagePlus size={15} />{loading ? 'Preparing image…' : 'Upload wallpaper'}
        </button>
        {preferences.customWallpaper && <button type="button" className="secondary" disabled={loading}
          onClick={() => onChange({ customWallpaper: '', ...(preferences.homeWallpaper === 'custom' ? { homeWallpaper: 'none' } : {}) })}>
          Remove my image
        </button>}
        <span className="appearance-preview-caption">JPEG, PNG, or WebP · Up to 16 MB</span>
      </div>
      {error && <p className="wallpaper-error" role="alert">{error}</p>}
    </div>
  </section>;
}

export function AppearanceSettings({ preferences, theme, onChange, onReset, saveError }: {
  preferences: AppearancePreferences; theme: ResolvedTheme; onChange: (patch: Partial<AppearancePreferences>) => void;
  onReset: () => void; saveError: string;
}) {
  const isDefault = (Object.keys(defaultAppearance) as (keyof AppearancePreferences)[]).every(key => key === 'lightColors' || key === 'darkColors' ? Object.keys(preferences[key]).length === 0 : preferences[key] === defaultAppearance[key]);
  return <div className="settings-page appearance-settings">
    <div className="page-heading"><div className="general-heading-line"><h1>Appearance</h1><span className="settings-device-label">This device</span></div><p>Personalize the workspace, code highlighting, reading layout, and motion.</p></div>
    {saveError && <p className="appearance-save-error" role="alert">{saveError}</p>}

    <section className="preference-section" aria-labelledby="appearance-theme-heading">
      <div className="preference-section-heading"><h2 id="appearance-theme-heading">App theme</h2><p>Choose a light or dark mode and a color for the whole workspace.</p></div>
      <div className="preference-group">
        <div className="theme-options" role="group" aria-label="Theme previews">
          {themes.map(({ value, label, icon: Icon }) => <button key={value} type="button" className={`theme-option ${preferences.theme === value ? 'is-selected' : ''}`} aria-label={`Use ${value} appearance`} aria-pressed={preferences.theme === value} onClick={() => onChange({ theme: value })}>
            <ThemePreview theme={value} /><span className="theme-option-label"><Icon size={16} /><span>{label}</span><span className="theme-option-check">{preferences.theme === value && <Check size={12} />}</span></span>
          </button>)}
        </div>
        <div className="preference-row">
          <div className="preference-copy">
            <label htmlFor="appearance-theme">Theme</label>
            <p id="appearance-theme-help">{preferences.theme === 'system' ? `Follows your system. Currently using ${theme} appearance.` : 'Use a fixed theme, or follow your system automatically.'}</p>
          </div>
          <div className="appearance-theme-select">
            <select id="appearance-theme" aria-describedby="appearance-theme-help" value={preferences.theme}
              onChange={event => onChange({ theme: event.target.value as ThemePreference })}>
              {themes.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
            </select>
            <ChevronDown size={16} strokeWidth={2.2} aria-hidden="true" />
          </div>
        </div>
        <div className="preference-row appearance-accent-row"><div className="preference-copy"><span id="appearance-accent-label">App color</span><p>Tints the workspace, sidebar, settings, code surfaces, and controls in both modes.</p></div><div className="appearance-accents" role="group" aria-labelledby="appearance-accent-label">
          {Object.entries(accentColors).map(([value, color]) => <button type="button" key={value} className="appearance-accent" aria-label={`${color.label} app color`} aria-pressed={preferences.accent === value && !preferences[theme === 'dark' ? 'darkColors' : 'lightColors']['--accent']} onClick={() => {
            const lightColors = { ...preferences.lightColors };
            const darkColors = { ...preferences.darkColors };
            delete lightColors['--accent'];
            delete darkColors['--accent'];
            onChange({ accent: value as AppearancePreferences['accent'], lightColors, darkColors });
          }}><span className="appearance-swatch" style={{ backgroundColor: color[theme], color: 'var(--appearance-swatch-ink)' }} aria-hidden="true">{preferences.accent === value && !preferences[theme === 'dark' ? 'darkColors' : 'lightColors']['--accent'] && <Check size={15} />}</span><span>{color.label}</span></button>)}
        </div></div>
      </div>
    </section>

    <HomeWallpaperSettings preferences={preferences} onChange={onChange} />

    <PaletteStudio preferences={preferences} theme={theme} onChange={onChange} />

    <section className="preference-section" aria-labelledby="appearance-text-heading">
      <div className="preference-section-heading"><h2 id="appearance-text-heading">Text &amp; readability</h2><p>Fine-tune conversations and code to suit your eyes.</p></div>
      <div className="preference-group">
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-font">Conversation font</label><p>Applies to messages and the composer.</p></div><select id="appearance-font" value={preferences.chatFont} onChange={event => onChange({ chatFont: event.target.value as AppearancePreferences['chatFont'] })}><option value="plex">IBM Plex Sans</option><option value="system">System font</option><option value="serif">IBM Plex Serif</option></select></div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-text-size">Text size</label><p>Adjust the size of conversation text.</p></div><div className="appearance-range"><input id="appearance-text-size" type="range" min="12" max="20" step="1" value={preferences.chatFontSize} aria-valuetext={`${preferences.chatFontSize} pixels`} onChange={event => onChange({ chatFontSize: Number(event.target.value) })} /><output htmlFor="appearance-text-size">{preferences.chatFontSize} px</output></div></div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-code-size">Code size</label><p>Applies to code blocks, the editor, and terminal.</p></div><select id="appearance-code-size" value={preferences.codeFontSize} onChange={event => onChange({ codeFontSize: Number(event.target.value) })}>{Array.from({ length: 8 }, (_, i) => i + 11).map(size => <option value={size} key={size}>{size} px</option>)}</select></div>
      </div>
      <div className="appearance-reading-preview" role="region" aria-label="Text preview"><span className="appearance-preview-caption">Text preview</span><p>A little more room to think.<br />Your next idea starts here.</p><CodeBlock code="const greeting = 'Hello, localLM';" language="typescript" /></div>
    </section>

    <section className="preference-section" aria-labelledby="appearance-layout-heading">
      <div className="preference-section-heading"><h2 id="appearance-layout-heading">Layout</h2><p>Find the right balance of focus and space.</p></div>
      <div className="preference-group">
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-density">Interface density</label><p>Adjust spacing in navigation, settings, and conversations.</p></div><select id="appearance-density" value={preferences.density} onChange={event => onChange({ density: event.target.value as AppearancePreferences['density'] })}><option value="comfortable">Comfortable</option><option value="compact">Compact</option></select></div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-width">Conversation width</label><p>Set the reading width for messages and the composer.</p></div><select id="appearance-width" value={preferences.chatWidth} onChange={event => onChange({ chatWidth: event.target.value as AppearancePreferences['chatWidth'] })}><option value="standard">Standard</option><option value="wide">Wide</option><option value="full">Full width</option></select></div>
      </div>
    </section>

    <section className="preference-section" aria-labelledby="appearance-motion-heading">
      <div className="preference-section-heading"><h2 id="appearance-motion-heading">Motion</h2><p>Keep movement at a comfortable level.</p></div>
      <div className="preference-group">
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-motion">Animations</label><p>System follows your device’s reduced motion preference.</p></div><select id="appearance-motion" value={preferences.motion} onChange={event => onChange({ motion: event.target.value as AppearancePreferences['motion'] })}><option value="system">Follow system</option><option value="reduced">Reduce motion</option></select></div>
        <div className="preference-row">
          <div className="preference-copy"><label htmlFor="appearance-motion-speed">Animation speed</label><p>Sets the pace of panel entrances, activity indicators, and common transitions. Reduced motion takes priority.</p></div>
          <select id="appearance-motion-speed" value={preferences.motionSpeed} onChange={event => onChange({ motionSpeed: event.target.value as AppearancePreferences['motionSpeed'] })}>
            <option value="fast">Fast</option><option value="standard">Standard</option><option value="slow">Slow</option>
          </select>
        </div>
      </div>
      <MotionPreview motion={preferences.motion} speed={preferences.motionSpeed} />
    </section>

    <footer className="appearance-footer"><p>{saveError ? 'Changes apply immediately. Preferences have not been saved.' : 'Changes apply immediately and are saved on this device.'}</p><button type="button" className="secondary" disabled={isDefault && !saveError} onClick={onReset}><RotateCcw size={14} />Reset appearance</button></footer>
  </div>;
}
