import { useEffect, useRef, useState, type ComponentType } from 'react';
import { Activity, BookOpen, CloudSun, Compass, Cpu, FileCode2, GitBranch, Lock, Monitor, Palette, Plug, Search, Settings, ShieldCheck, SlidersHorizontal, SquareTerminal, Terminal, X } from 'lucide-react';
import type { Page } from './Sidebar';

export const settingsItems: { page: Page; label: string; group: string; keywords: string; icon: ComponentType<{ size?: number }>; sub?: boolean }[] = [
  { page: 'general', label: 'General', group: 'General', keywords: 'permissions access folder project', icon: Settings },
  { page: 'appearance', label: 'Appearance', group: 'Appearance', keywords: 'theme light dark system accent color font text code size density spacing width animations motion display readability', icon: Palette },
  { page: 'models', label: 'Models & runtime', group: 'Models & providers', keywords: 'local gguf llama gpu cpu generation diagnostics download provider api key custom openai claude endpoint ollama', icon: Cpu },
  { page: 'inference', label: 'Inference', group: 'Models & providers', keywords: 'sampling temperature top p tokens response system prompt', icon: SlidersHorizontal },
  { page: 'tools', label: 'Tools & permissions', group: 'Tools & permissions', keywords: 'approval access workspace files security', icon: ShieldCheck },
  { page: 'permission-rules', label: 'Permission rules', group: 'Tools & permissions', keywords: 'allow deny ask rule command path host approval access', icon: ShieldCheck, sub: true },
  { page: 'security', label: 'Security', group: 'Tools & permissions', keywords: 'offline external retrieval permissions privacy approval local data', icon: Lock },
  { page: 'execution', label: 'Execution', group: 'Execution', keywords: 'terminal python node powershell interpreters web search', icon: Terminal },
  { page: 'bash', label: 'Bash', group: 'Execution', keywords: 'shell commands terminal environment timeout approval executable', icon: SquareTerminal, sub: true },
  { page: 'python', label: 'Python', group: 'Execution', keywords: 'scripts virtual environment venv interpreter timeout approval', icon: FileCode2, sub: true },
  { page: 'language-servers', label: 'Language servers', group: 'Execution', keywords: 'lsp diagnostics symbols references rename executable', icon: FileCode2, sub: true },
  { page: 'hooks', label: 'Hooks', group: 'Execution', keywords: 'hooks pre post tool use shell commands', icon: SquareTerminal, sub: true },
  { page: 'development-tools', label: 'Development tools', group: 'Execution', keywords: 'debugger profiling database postgres mysql http credentials openapi', icon: SquareTerminal, sub: true },
  { page: 'web-search', label: 'Web Search', group: 'Execution', keywords: 'searxng search retrieval cache fast balanced deep connection provider', icon: Search, sub: true },
  { page: 'weather', label: 'Weather', group: 'Execution', keywords: 'forecast temperature celsius fahrenheit location units', icon: CloudSun, sub: true },
  { page: 'browser-use', label: 'Browser Use', group: 'Execution', keywords: 'automation playwright chrome edge headless websites tabs engine', icon: Compass, sub: true },
  { page: 'computer-use', label: 'Computer Use', group: 'Execution', keywords: 'automation desktop monitor screenshots mouse keyboard vision x11 wayland', icon: Monitor, sub: true },
  { page: 'connectors', label: 'Connectors', group: 'Connectors & skills', keywords: 'mcp servers integrations accounts', icon: Plug },
  { page: 'skills', label: 'Skills', group: 'Connectors & skills', keywords: 'instructions workflows packages', icon: BookOpen },
  { page: 'plugins', label: 'Plugin tools', group: 'Connectors & skills', keywords: 'plugin command tool extension custom deploy', icon: Plug, sub: true },
  { page: 'git', label: 'Git', group: 'Connectors & skills', keywords: 'repository branch stage unstage commit history diff', icon: GitBranch },
  { page: 'diagnostics', label: 'Diagnostics', group: 'Advanced', keywords: 'logs runtime gpu cpu hardware errors troubleshooting', icon: Activity },
];

const settingsGroups = ['General', 'Appearance', 'Models & providers', 'Tools & permissions', 'Execution', 'Connectors & skills', 'Advanced'];

export function SettingsNavigation({ page, onPage, onClose }: { page: Page; onPage: (page: Page) => void; onClose: () => void }) {
  const [query, setQuery] = useState('');
  const input = useRef<HTMLInputElement>(null);
  const navigation = useRef<HTMLElement>(null);
  useEffect(() => {
    const revealSelected = () => navigation.current?.querySelector('[aria-current="page"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    revealSelected();
    window.addEventListener('resize', revealSelected);
    return () => window.removeEventListener('resize', revealSelected);
  }, [page, query]);
  const filtered = settingsItems.filter(item => `${item.label} ${item.keywords}`.toLowerCase().includes(query.trim().toLowerCase()));
  return <aside className="settings-sidebar" aria-label="Settings">
    <div className="settings-sidebar-heading"><h2>Settings</h2><button className="icon-button" aria-label="Close settings" title="Back to conversation" onClick={onClose}><X size={18} /></button></div>
    <div className="settings-search"><Search size={17} /><input ref={input} type="search" aria-label="Search settings" placeholder="Search settings" value={query} onChange={event => setQuery(event.target.value)} />{query && <button aria-label="Clear settings search" onClick={() => { setQuery(''); input.current?.focus(); }}><X size={15} /></button>}</div>
    <nav ref={navigation} className="settings-navigation" aria-label="Settings navigation">
      {settingsGroups.map(group => {
        const items = filtered.filter(item => item.group === group);
        return items.length > 0 && <div className="settings-navigation-group" key={group}><p>{group}</p>{items.map(({ page: target, label, icon: Icon, sub }) => <button key={target} className={`${page === target ? 'selected' : ''} ${sub ? 'settings-child' : ''}`} aria-label={`Settings: ${target === 'tools' ? 'Tools' : label}`} aria-current={page === target ? 'page' : undefined} onClick={() => onPage(target)}><Icon size={18} /><span>{label}</span></button>)}</div>;
      })}
      {!filtered.length && <p className="settings-no-results" role="status">No settings found for “{query}”.<button onClick={() => setQuery('')}>Clear search</button></p>}
    </nav>
  </aside>;
}
