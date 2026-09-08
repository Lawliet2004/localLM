import { BookOpen, Cpu, MessageSquare, PanelLeftClose, Plug, Plus, Search, Sun, Moon, Terminal } from 'lucide-react';
import type { Conversation } from '../lib/types';
export type Page = 'chat' | 'models' | 'connectors' | 'skills' | 'execution';
interface Props {
  page: Page; onPage: (page: Page) => void; conversations: Conversation[]; activeId: string | null;
  onSelect: (id: string) => void; onNew: () => void; busy: boolean; search: string; onSearch: (search: string) => void;
  theme: string; onTheme: () => void; onCollapse: () => void;
}
export function Sidebar(props: Props) {
  return <aside className="sidebar"><div className="brand"><span className="brand-symbol">L<span /></span><strong>LocalLM<span>WORKSPACE</span></strong><button className="icon-button" aria-label="Collapse sidebar" onClick={props.onCollapse}><PanelLeftClose size={17} /></button></div>
    <button className="new-chat" disabled={props.busy} onClick={props.onNew}><Plus size={17} /><span>New conversation</span><kbd>Ctrl N</kbd></button>
    <nav className="main-nav" aria-label="Workspace">{([{ page: 'chat', title: 'Conversations', icon: MessageSquare }, { page: 'connectors', title: 'Connectors', icon: Plug }, { page: 'skills', title: 'Skills', icon: BookOpen }, { page: 'execution', title: 'Execution', icon: Terminal }, { page: 'models', title: 'Models & runtime', icon: Cpu }] as const).map(({ page, title, icon: Icon }) => <button key={page} className={props.page === page ? 'selected' : ''} onClick={() => props.onPage(page)}><Icon size={16} /><span>{title}</span></button>)}</nav>
    <div className="history-heading"><span>RECENT</span><span>{props.conversations.length}</span></div><label className="search-field sidebar-search"><Search size={14} /><input type="search" aria-label="Search conversations" placeholder="Find a conversation" value={props.search} onChange={e => props.onSearch(e.target.value)} /></label>
    <div className="conversation-list">{props.conversations.filter(item => item.title.toLowerCase().includes(props.search.toLowerCase())).map(item => <button key={item.id} className={props.activeId === item.id && props.page === 'chat' ? 'active-conversation' : ''} disabled={props.busy} onClick={() => props.onSelect(item.id)} title={item.title}><span>{item.title}</span></button>)}{!props.conversations.length && <p className="history-empty">Your conversations will<br />find a home here.</p>}</div>
    <div className="sidebar-footer"><span><span className="local-orbit" />Local workspace<small>Stored on this device</small></span><button className="icon-button" aria-label={`Use ${props.theme === 'dark' ? 'light' : 'dark'} theme`} onClick={props.onTheme}>{props.theme === 'dark' ? <Sun size={17} /> : <Moon size={17} />}</button></div>
  </aside>;
}
