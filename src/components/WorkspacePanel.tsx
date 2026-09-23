import { useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { File, Folder, FolderOpen, ListTodo, PanelRight, PanelBottom, Terminal, Maximize2, Minimize2, Globe, Plus, X } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { TerminalPanel, closeTerminalForTab } from './TerminalPanel';
import { BrowserPanel } from './BrowserPanel';
import { WorkspaceTree, FileView } from './FilesPanel';

type TabKind = 'review' | 'terminal' | 'browser' | 'files' | 'file';
interface WorkspaceTab {
  id: string;
  kind: TabKind;
  title: string;
  path?: string;
}

const LAUNCHERS = [
  { kind: 'review', label: 'Review', icon: ListTodo, shortcut: 'Ctrl+Shift+G' },
  { kind: 'terminal', label: 'Terminal', icon: Terminal, shortcut: 'Ctrl+`' },
  { kind: 'browser', label: 'Browser', icon: Globe, shortcut: 'Ctrl+T' },
  { kind: 'files', label: 'Files', icon: FolderOpen, shortcut: 'Ctrl+P' },
] as const;
// Review and the open-file picker are singletons; terminals, browsers, and
// opened files multiply like real editor tabs.
const SINGLETONS = new Set<TabKind>(['review', 'files']);
let tabSeq = 0;

function makeTab(kind: TabKind, patch: Partial<WorkspaceTab> = {}): WorkspaceTab {
  const id = `${kind}-${++tabSeq}-${Math.random().toString(36).slice(2, 7)}`;
  const titles: Record<TabKind, string> = { review: 'Review', terminal: 'Terminal', browser: 'New tab', files: 'Open file', file: 'File' };
  return { id, kind, title: titles[kind], ...patch };
}

const TAB_ICONS = { review: ListTodo, terminal: Terminal, browser: Globe, files: FolderOpen, file: File } as const;

export function WorkspacePanel({ busy, revision }: { busy: boolean; revision: string }) {
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = tabs.find(t => t.id === activeId) ?? null;

  const [root, setRoot] = useState('');
  const [git, setGit] = useState<{ branch: string; branches: string[]; status: string; diff: string } | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  async function act(action: () => Promise<unknown>) {
    setLoading(true); setError('');
    try { await action(); } catch (e) { setError(errorMessage(e)); } finally { setLoading(false); }
  }

  function openKind(kind: TabKind, fresh: boolean) {
    setOpen(true);
    const existing = !fresh || SINGLETONS.has(kind) ? tabs.find(t => t.kind === kind) : undefined;
    if (existing) {
      setActiveId(existing.id);
      return;
    }
    const tab = makeTab(kind);
    setTabs(current => [...current, tab]);
    setActiveId(tab.id);
  }

  function openFile(path: string) {
    const existing = tabs.find(t => t.kind === 'file' && t.path === path);
    if (existing) { setActiveId(existing.id); return; }
    const tab = makeTab('file', { title: path.split('/').pop() ?? path, path });
    setTabs(current => [...current, tab]);
    setActiveId(tab.id);
  }

  function updateTab(id: string, patch: Partial<WorkspaceTab>) {
    setTabs(current => current.map(t => t.id === id ? { ...t, ...patch } : t));
  }

  function closeTab(tab: WorkspaceTab) {
    if (tab.kind === 'terminal') closeTerminalForTab(tab.id);
    if (tab.kind === 'browser') void api.browserClose(tab.id).catch(() => {});
    setTabs(current => {
      const index = current.findIndex(t => t.id === tab.id);
      const next = current.filter(t => t.id !== tab.id);
      if (activeId === tab.id) setActiveId(next.length ? next[Math.max(0, index - 1)].id : null);
      return next;
    });
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // A focused terminal owns its keystrokes (Ctrl+P history, Escape in vim).
      if ((event.target as HTMLElement | null)?.closest?.('.terminal-host')) return;
      if (event.key === 'Escape') { setOpen(false); return; }
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      const kind: TabKind | null = key === 'g' && event.shiftKey ? 'review' : !event.shiftKey && key === '`' ? 'terminal' : !event.shiftKey && key === 'p' ? 'files' : !event.shiftKey && key === 't' ? 'browser' : null;
      if (kind) { event.preventDefault(); openKind(kind, false); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getWorkspace().then(w => { if (!disposed) setRoot(w.path); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [revision]);

  useEffect(() => {
    if (!nativeAvailable || !open || active?.kind !== 'review' || !root) { setGit(null); return; }
    let cancelled = false;
    setLoading(true);
    setError('');
    api.workspaceGit()
      .then(result => { if (!cancelled) setGit(result); })
      .catch(e => { if (!cancelled) setError(errorMessage(e)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, active?.id, root, revision]);

  async function choose() {
    const selected = await openDialog({ directory: true, title: 'Choose workspace' });
    if (typeof selected === 'string') { await api.setWorkspace(selected); setRoot(selected); }
  }

  const fileContext = active?.kind === 'files' || active?.kind === 'file';

  return <div className="workspace-panel-container">
    <div className="panel-toggles">
      {open && <button className="icon-button" title={expanded ? 'Restore panel size' : 'Expand panel'} aria-label={expanded ? 'Restore panel size' : 'Expand panel'} aria-pressed={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? <Minimize2 size={17} /> : <Maximize2 size={17} />}</button>}
      <button className="icon-button" title="Review changes (Ctrl+Shift+G)" aria-label="Review changes" aria-pressed={open && active?.kind === 'review'} onClick={() => openKind('review', false)}><ListTodo size={18} /></button>
      <button className="icon-button" title="Terminal (Ctrl+`)" aria-label="Toggle terminal" aria-pressed={open && active?.kind === 'terminal'} onClick={() => openKind('terminal', false)}><PanelBottom size={17} /></button>
      <button className="icon-button" title="Workspace panel" aria-label="Toggle workspace panel" aria-expanded={open} onClick={() => setOpen(!open)}><PanelRight size={17} /></button>
    </div>
    {open && <aside className={`workspace-panel ${expanded ? 'panel-expanded' : ''}`} aria-label="Workspace inspector">
      <div className="workspace-tabs">
        {tabs.map(tab => {
          const Icon = TAB_ICONS[tab.kind];
          return <button key={tab.id} className={`workspace-tab ${tab.id === activeId ? 'active' : ''}`} onClick={() => setActiveId(tab.id)} title={tab.path ?? tab.title}>
            <Icon size={13} className="workspace-tab-icon" />
            <span className="workspace-tab-title">{tab.title}</span>
            <span className="workspace-tab-close" role="button" aria-label={`Close ${tab.title}`} onClick={event => { event.stopPropagation(); closeTab(tab); }}><X size={12} /></span>
          </button>;
        })}
        <details className="app-menu workspace-add" name="application-menu">
          <summary className="icon-button" aria-label="New workspace tab" title="New workspace tab"><Plus size={16} /></summary>
          <nav className="menu" aria-label="Workspace shortcuts">
            {LAUNCHERS.map(item => <button key={item.kind} onClick={event => { event.currentTarget.closest('details')?.removeAttribute('open'); openKind(item.kind, true); }}><item.icon size={15} /><span>{item.label}</span><kbd>{item.shortcut}</kbd></button>)}
          </nav>
        </details>
        <div className="workspace-tabs-spacer" />
        <div className="workspace-tab-actions">
          <button className="icon-button" aria-label={expanded ? 'Restore panel size' : 'Expand panel'} aria-pressed={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button>
          <button className="icon-button" aria-label="Review changes" aria-pressed={active?.kind === 'review'} onClick={() => openKind('review', false)}><ListTodo size={16} /></button>
          <button className="icon-button" aria-label="Open terminal in panel" aria-pressed={active?.kind === 'terminal'} onClick={() => openKind('terminal', false)}><PanelBottom size={16} /></button>
          <button className="icon-button selected" aria-label="Close inspector" onClick={() => setOpen(false)}><PanelRight size={16} /></button>
        </div>
      </div>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <div className="workspace-body">
        <div className="workspace-tab-content">
          {tabs.length === 0 && <nav className="workspace-empty" aria-label="Workspace shortcuts">
            {LAUNCHERS.map(item => <button key={item.kind} onClick={() => openKind(item.kind, false)}><item.icon size={17} /><span>{item.label}</span><kbd>{item.shortcut}</kbd></button>)}
          </nav>}
          {active?.kind === 'terminal' && <TerminalPanel key={active.id} tabKey={active.id} />}
          {active?.kind === 'browser' && <BrowserPanel tabId={active.id} onError={setError} onNavigate={url => {
            let host = 'New tab';
            try { host = new URL(url).hostname || 'New tab'; } catch { /* keep */ }
            updateTab(active.id, { title: host });
          }} />}
          {active?.kind === 'review' && <div className="review-view">
            <div className="panel-workspace-actions">
              <button className="project-chip" disabled={busy || !nativeAvailable} title={root || 'Choose workspace'} onClick={() => void act(choose)}><Folder size={15} />{root ? root.split(/[\\/]/).pop() : 'Choose workspace'}</button>
              <button className="icon-button" aria-label="Open workspace folder" disabled={!root || !nativeAvailable} onClick={() => void act(() => openPath(root))}><FolderOpen size={17} /></button>
            </div>
            {loading && <p role="status">Loading…</p>}
            {!root && <p className="panel-empty">Choose a workspace to review its changes.</p>}
            {git && <>
              <label className="panel-toolbar">Branch<select aria-label="Switch branch" value={git.branch} disabled={busy || loading} onChange={e => void act(async () => setGit(await api.workspaceGit(e.target.value)))}>{!git.branch && <option value="">Detached HEAD</option>}{git.branches.map(b => <option key={b}>{b}</option>)}</select></label>
              <pre>{git.status || 'Working tree clean'}</pre>
              <pre>{git.diff || 'No changes to commit.'}</pre>
            </>}
          </div>}
          {active?.kind === 'files' && <div className="file-open-empty">
            <FolderOpen size={30} />
            <h2>Open file</h2>
            <p>Select a file from the workspace tree</p>
          </div>}
          {active?.kind === 'file' && active.path && <FileView root={root} path={active.path} onError={setError} />}
        </div>
        <WorkspaceTree root={root} hidden={!fileContext} onOpenFile={openFile} onChooseWorkspace={() => void act(choose)} onError={setError} />
      </div>
    </aside>}
  </div>;
}
