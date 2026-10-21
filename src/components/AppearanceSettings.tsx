import { Check, Monitor, Moon, RotateCcw, Sun } from 'lucide-react';
import { accentColors, defaultAppearance, type AppearancePreferences, type ResolvedTheme, type ThemePreference } from '../lib/appearance';
import './AppearanceSettings.css';

const themes = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
  { value: 'system', label: 'System', icon: Monitor },
] as const;

function ThemePreview({ theme }: { theme: ThemePreference }) {
  return <span className={`theme-preview theme-preview-${theme}`} aria-hidden="true"><span className="theme-preview-rail"><i /><i /><i /></span><span className="theme-preview-sidebar"><i /><i /><i /></span><span className="theme-preview-canvas"><span /><span /><span className="theme-preview-composer" /></span></span>;
}

export function AppearanceSettings({ preferences, theme, onChange, onReset, saveError }: {
  preferences: AppearancePreferences; theme: ResolvedTheme; onChange: (patch: Partial<AppearancePreferences>) => void;
  onReset: () => void; saveError: string;
}) {
  const isDefault = (Object.keys(defaultAppearance) as (keyof AppearancePreferences)[]).every(key => preferences[key] === defaultAppearance[key]);
  return <div className="settings-page appearance-settings">
    <div className="page-heading"><div className="general-heading-line"><h1>Appearance</h1><span className="settings-device-label">This device</span></div><p>Make your workspace comfortable, down to the details.</p></div>
    {saveError && <p className="appearance-save-error" role="alert">{saveError}</p>}

    <section className="preference-section" aria-labelledby="appearance-theme-heading">
      <div className="preference-section-heading"><h2 id="appearance-theme-heading">Theme &amp; color</h2><p>Choose the backdrop for your work.</p></div>
      <div className="preference-group">
        <div className="theme-options" role="group" aria-label="Theme previews">
          {themes.map(({ value, label, icon: Icon }) => <button key={value} type="button" className={`theme-option ${preferences.theme === value ? 'is-selected' : ''}`} aria-label={`Use ${value} appearance`} aria-pressed={preferences.theme === value} onClick={() => onChange({ theme: value })}>
            <ThemePreview theme={value} /><span className="theme-option-label"><Icon size={16} /><span>{label}</span><span className="theme-option-check">{preferences.theme === value && <Check size={12} />}</span></span>
          </button>)}
        </div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-theme">Theme</label><p id="appearance-theme-help">{preferences.theme === 'system' ? `Follows your system. Currently using ${theme} appearance.` : 'Use a fixed theme, or follow your system automatically.'}</p></div><select id="appearance-theme" aria-describedby="appearance-theme-help" value={preferences.theme} onChange={event => onChange({ theme: event.target.value as ThemePreference })}><option value="light">Light</option><option value="dark">Dark</option><option value="system">System</option></select></div>
        <div className="preference-row appearance-accent-row"><div className="preference-copy"><span id="appearance-accent-label">Accent color</span><p>Used for selections, buttons, and keyboard focus.</p></div><div className="appearance-accents" role="group" aria-labelledby="appearance-accent-label">
          {Object.entries(accentColors).map(([value, color]) => <button type="button" key={value} className="appearance-accent" aria-label={`${color.label} accent`} aria-pressed={preferences.accent === value} onClick={() => onChange({ accent: value as AppearancePreferences['accent'] })}><span className="appearance-swatch" style={{ backgroundColor: color[theme], color: theme === 'dark' ? '#202b21' : '#fff' }} aria-hidden="true">{preferences.accent === value && <Check size={15} />}</span><span>{color.label}</span></button>)}
        </div></div>
      </div>
    </section>

    <section className="preference-section" aria-labelledby="appearance-text-heading">
      <div className="preference-section-heading"><h2 id="appearance-text-heading">Text &amp; readability</h2><p>Fine-tune conversations and code to suit your eyes.</p></div>
      <div className="preference-group">
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-font">Conversation font</label><p>Applies to messages and the composer.</p></div><select id="appearance-font" value={preferences.chatFont} onChange={event => onChange({ chatFont: event.target.value as AppearancePreferences['chatFont'] })}><option value="plex">IBM Plex Sans</option><option value="system">System font</option><option value="serif">IBM Plex Serif</option></select></div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-text-size">Text size</label><p>Adjust the size of conversation text.</p></div><div className="appearance-range"><input id="appearance-text-size" type="range" min="12" max="20" step="1" value={preferences.chatFontSize} aria-valuetext={`${preferences.chatFontSize} pixels`} onChange={event => onChange({ chatFontSize: Number(event.target.value) })} /><output htmlFor="appearance-text-size">{preferences.chatFontSize} px</output></div></div>
        <div className="preference-row"><div className="preference-copy"><label htmlFor="appearance-code-size">Code size</label><p>Applies to code blocks, the editor, and terminal.</p></div><select id="appearance-code-size" value={preferences.codeFontSize} onChange={event => onChange({ codeFontSize: Number(event.target.value) })}>{Array.from({ length: 8 }, (_, i) => i + 11).map(size => <option value={size} key={size}>{size} px</option>)}</select></div>
      </div>
      <div className="appearance-reading-preview" role="region" aria-label="Text preview"><span className="appearance-preview-caption">Text preview</span><p>A little more room to think.<br />Your next idea starts here.</p><pre><code>const greeting = 'Hello, localLM';</code></pre></div>
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
      </div>
    </section>

    <footer className="appearance-footer"><p>{saveError ? 'Changes apply immediately. Preferences have not been saved.' : 'Changes apply immediately and are saved on this device.'}</p><button type="button" className="secondary" disabled={isDefault && !saveError} onClick={onReset}><RotateCcw size={14} />Reset appearance</button></footer>
  </div>;
}
