// Floating pill dock: the app's primary navigation, detached from the
// window edges. Vertical pill centered on the left at desktop widths,
// horizontal pill centered along the bottom below 760px.
import type { ComponentType } from 'react';
import { House, Moon, Settings, ShieldCheck, Sun } from 'lucide-react';
import type { Page } from './Sidebar';
import { settingsItems } from './SettingsNavigation';

const dockPages: Page[] = ['models', 'execution', 'connectors', 'skills'];
const settingsPages: Page[] = ['general', 'appearance', 'tools', 'inference', 'diagnostics', 'security', 'git', 'bash', 'python', 'weather', 'web-search', 'browser-use', 'computer-use'];
const dockItems: { page: Page; label: string; icon: ComponentType<{ size?: number }> }[] =
  settingsItems.filter(item => dockPages.includes(item.page));

export function TabDock({ page, onPage, theme, onTheme }: {
  page: Page; onPage: (page: Page) => void; theme: string; onTheme: () => void;
}) {
  const themeLabel = `Use ${theme === 'dark' ? 'light' : 'dark'} theme`;
  return <nav className="app-dock" aria-label="Main navigation">
    <button className="dock-logo" aria-label="LocalLM home" onClick={() => onPage('chat')} data-tooltip="LocalLM">
      <svg viewBox="0 0 24 24" width="21" height="21" aria-hidden="true"><path d="M5 4v16h14" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round" /><circle cx="17" cy="7" r="3" fill="var(--accent)" /></svg>
    </button>
    <span className="dock-divider" aria-hidden="true" />
    <button aria-label="Conversations" aria-current={page === 'chat' ? 'page' : undefined} onClick={() => onPage('chat')} data-tooltip="Conversations"><House size={19} /></button>
    {dockItems.map(({ page: target, label, icon: Icon }) => <button key={target} aria-label={label} aria-current={page === target ? 'page' : undefined} onClick={() => onPage(target)} data-tooltip={label}><Icon size={19} /></button>)}
    <button aria-label="Tools" aria-current={page === 'tools' ? 'page' : undefined} onClick={() => onPage('tools')} data-tooltip="Tools & permissions"><ShieldCheck size={19} /></button>
    <span className="dock-spacer" aria-hidden="true" />
    <span className="dock-divider" aria-hidden="true" />
    <button aria-label={themeLabel} onClick={onTheme} data-tooltip={themeLabel}>{theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}</button>
    <button aria-label="Open settings" aria-current={settingsPages.includes(page) ? 'page' : undefined} onClick={() => onPage('general')} data-tooltip="Settings"><Settings size={19} /></button>
  </nav>;
}
