import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { Brain, ChevronRight, File, Folder, FolderOpen, ListTodo, PanelRight, Terminal, Maximize2, Minimize2, Globe, Plus, X } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { BrowserPanel, closeBrowserTab } from './BrowserPanel';
import { TaskReview } from './TaskReview';
import { ContextPanel } from './ContextPanel';
import type { TaskWorkspace } from '../lib/types';

const TerminalPanel = lazy(() => import('./TerminalPanel').then(module => ({ default: module.TerminalPanel })));
const WorkspaceTree = lazy(() => import('./FilesPanel').then(module => ({ default: module.WorkspaceTree })));
const FileView = lazy(() => import('./FilesPanel').then(module => ({ default: module.FileView })));

type TabKind = 'review' | 'terminal' | 'browser' | 'files' | 'file' | 'context';
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
  { kind: 'context', label: 'Context', icon: Brain, shortcut: '' },
] as const;
// Review and the open-file picker are singletons; terminals, browsers, and
// opened files multiply like real editor tabs.
const SINGLETONS = new Set<TabKind>(['review', 'files', 'context']);
let tabSeq = 0;

function makeTab(kind: TabKind, patch: Partial<WorkspaceTab> = {}): WorkspaceTab {
  const id = `${kind}-${++tabSeq}-${Math.random().toString(36).slice(2, 7)}`;
  const titles: Record<TabKind, string> = { review: 'Review', terminal: 'Terminal', browser: 'New tab', files: 'Open file', file: 'File', context: 'Context' };
  return { id, kind, title: titles[kind], ...patch };
}

const TAB_ICONS = { review: ListTodo, terminal: Terminal, browser: Globe, files: FolderOpen, file: File, context: Brain } as const;

export function WorkspacePanel({ busy, revision, conversation, taskTick = 0, onTaskStatus }: {
  busy: boolean;
  revision: string;
  /** Conversation the review tab's task workspace belongs to. */
  conversation?: string | null;
  /** Bump to re-fetch the task review (e.g. when a turn lands messages). */
  taskTick?: number;
  /** Mirrors the task workspace status up so the composer stays in sync. */
  onTaskStatus?: (task: TaskWorkspace | null) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [width, setWidth] = useState(() => {
    try {
      const saved = Number(localStorage.getItem('workspace-inspector-width'));
      return saved >= 320 && saved <= 840 ? saved : 440;
    } catch { return 440; }
  });
  const [workspaceWidth, setWorkspaceWidth] = useState(window.innerWidth);
  const effectiveWidth = Math.max(320, Math.min(width, 840, workspaceWidth - 420));
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = tabs.find(t => t.id === activeId) ?? null;
  const [browserReady, setBrowserReady] = useState(!nativeAvailable);

  const [root, setRoot] = useState('');
  const [git, setGit] = useState<{ branch: string; branches: string[]; status: string; diff: string } | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);

  function clampWidth(value: number) {
    return Math.max(320, Math.min(Math.round(value), 840, workspaceWidth - 420));
  }

  useEffect(() => {
    const workspace = containerRef.current?.closest<HTMLElement>('.workspace');
    if (!workspace) return;
    workspace.style.setProperty('--inspector-width', open && !expanded ? `${effectiveWidth}px` : '0px');
    return () => { workspace.style.removeProperty('--inspector-width'); };
  }, [open, expanded, effectiveWidth]);

  useEffect(() => {
    try { localStorage.setItem('workspace-inspector-width', String(width)); } catch { /* Storage can be disabled. */ }
  }, [width]);

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    void api.browserCloseAll().catch(() => {}).finally(() => { if (!disposed) setBrowserReady(true); });
    // The agent's browser tool created its webview: surface it as a tab so
    // the user can watch (and take over) what the agent is doing.
    const unlisten = listen<{ tab: string }>('agent-browser-opened', event => {
      const id = event.payload.tab;
      setOpen(true);
      setTabs(current => current.some(t => t.id === id) ? current : [...current, { id, kind: 'browser', title: 'Agent browser' }]);
      setActiveId(id);
    }).catch(() => () => {});
    return () => { disposed = true; void unlisten.then(stop => stop()); };
  }, []);

  useEffect(() => {
    const workspace = containerRef.current?.closest<HTMLElement>('.workspace');
    const measure = () => setWorkspaceWidth(workspace?.clientWidth || window.innerWidth);
    measure();
    const observer = workspace && typeof ResizeObserver !== 'undefined' ? new ResizeObserver(measure) : null;
    if (workspace) observer?.observe(workspace);
    window.addEventListener('resize', measure);
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, []);

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

  function closePanel() { setOpen(false); setExpanded(false); }

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
    if (tab.kind === 'terminal') void import('./TerminalPanel').then(module => module.closeTerminalForTab(tab.id)).catch(e => setError(errorMessage(e)));
    if (tab.kind === 'browser') void closeBrowserTab(tab.id).catch(() => {});
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
      if (event.key === 'Escape') { closePanel(); return; }
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

  return <div ref={containerRef} className="workspace-panel-container">
    <div className="panel-toggles">
      {open && <button className="icon-button" title={expanded ? 'Restore panel size' : 'Expand panel'} aria-label={expanded ? 'Restore panel size' : 'Expand panel'} aria-pressed={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? <Minimize2 size={17} /> : <Maximize2 size={17} />}</button>}
      <button className="icon-button" title="Review changes (Ctrl+Shift+G)" aria-label="Review changes" aria-pressed={open && active?.kind === 'review'} onClick={() => openKind('review', false)}><ListTodo size={18} /></button>
      <button className="icon-button" title="Workspace panel" aria-label="Toggle workspace panel" aria-expanded={open} onClick={() => open ? closePanel() : setOpen(true)}><PanelRight size={17} /></button>
    </div>
    {open && <aside className={`workspace-panel ${expanded ? 'panel-expanded' : ''}`} aria-label="Workspace inspector" style={expanded ? undefined : { width: effectiveWidth }}>
      {!expanded && <>
        <div className="workspace-panel-resize" role="separator" aria-label="Resize inspector" aria-orientation="vertical" aria-valuemin={320} aria-valuemax={Math.max(320, Math.min(840, workspaceWidth - 420))} aria-valuenow={effectiveWidth} tabIndex={0}
          onPointerDown={event => { drag.current = { x: event.clientX, width: effectiveWidth }; event.currentTarget.setPointerCapture(event.pointerId); }}
          onPointerMove={event => { if (drag.current) setWidth(clampWidth(drag.current.width + drag.current.x - event.clientX)); }}
          onPointerUp={event => { drag.current = null; if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId); }}
          onPointerCancel={() => { drag.current = null; }}
          onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); setWidth(current => clampWidth(current + (event.key === 'ArrowLeft' ? 24 : -24))); } }} />
        <button type="button" className="separator-collapse" aria-label="Collapse panel" title="Collapse panel" onClick={closePanel}><ChevronRight size={13} /></button>
      </>}
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
          <button className="icon-button selected" aria-label="Close inspector" onClick={closePanel}><PanelRight size={16} /></button>
        </div>
      </div>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <div className="workspace-body">
        <Suspense fallback={<p className="panel-empty" role="status">Opening workspace tab…</p>}>
        <div className="workspace-tab-content">
          {tabs.length === 0 && <nav className="workspace-empty" aria-label="Workspace shortcuts">
            {LAUNCHERS.map(item => <button key={item.kind} onClick={() => openKind(item.kind, false)}><item.icon size={17} /><span>{item.label}</span><kbd>{item.shortcut}</kbd></button>)}
          </nav>}
          {active?.kind === 'terminal' && <TerminalPanel key={active.id} tabKey={active.id} />}
          {active?.kind === 'browser' && browserReady && <BrowserPanel tabId={active.id} onError={setError} onNavigate={url => {
            let host = 'New tab';
            try { host = new URL(url).hostname || 'New tab'; } catch { /* keep */ }
            updateTab(active.id, { title: host });
          }} />}
          {active?.kind === 'context' && <ContextPanel conversation={conversation} />}
          {active?.kind === 'review' && <div className="review-view">
            <div className="panel-workspace-actions">
              <button className="project-chip" disabled={busy || !nativeAvailable} title={root || 'Choose workspace'} onClick={() => void act(choose)}><Folder size={15} />{root ? root.split(/[\\/]/).pop() : 'Choose workspace'}</button>
              <button className="icon-button" aria-label="Open workspace folder" disabled={!root || !nativeAvailable} onClick={() => void act(() => openPath(root))}><FolderOpen size={17} /></button>
            </div>
            {loading && <p role="status">Loading…</p>}
            {conversation && <TaskReview conversation={conversation} refreshTick={taskTick} onStatusChanged={task => onTaskStatus?.(task)} />}
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
        {tabs.some(tab => tab.kind === 'files' || tab.kind === 'file') && <WorkspaceTree root={root} hidden={!fileContext} onOpenFile={openFile} onChooseWorkspace={() => void act(choose)} onError={setError} />}
        </Suspense>
      </div>
    </aside>}
  </div>;
}
