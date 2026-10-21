import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { File, FolderOpen, ListTodo, PanelRight, Terminal, Maximize2, Minimize2, Globe, Plus, X } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { EditorMutation } from '../lib/api';
import { TerminalPanel, closeTerminalForTab } from './TerminalPanel';
import { BrowserPanel, closeBrowserForTab } from './BrowserPanel';
import { WorkspaceTree } from './EditorTree';
import type { TreeAction } from './EditorTree';
import { FileEditor } from './FileEditor';
import type { EditorHandle } from './FileEditor';
import { WorkspaceDialog } from './WorkspaceDialog';
import type { DialogOptions, DialogResult } from './WorkspaceDialog';
import { ReviewPanel } from './ReviewPanel';

type TabKind = 'review' | 'terminal' | 'browser' | 'files' | 'file';
interface WorkspaceTab { id: string; kind: TabKind; title: string; path?: string; root?: string; dirty?: boolean }
const LAUNCHERS = [
  { kind: 'review', label: 'Review', icon: ListTodo, shortcut: 'Ctrl+Shift+G' },
  { kind: 'terminal', label: 'Terminal', icon: Terminal, shortcut: 'Ctrl+`' },
  { kind: 'browser', label: 'Browser', icon: Globe, shortcut: 'Ctrl+T' },
  { kind: 'files', label: 'Editor', icon: FolderOpen, shortcut: 'Ctrl+P' },
] as const;
const SINGLETONS = new Set<TabKind>(['review', 'files']);
const TAB_ICONS = { review: ListTodo, terminal: Terminal, browser: Globe, files: FolderOpen, file: File } as const;
let tabSeq = 0;
function makeTab(kind: TabKind, patch: Partial<WorkspaceTab> = {}): WorkspaceTab {
  const titles = { review: 'Review', terminal: 'Terminal', browser: 'New tab', files: 'Editor', file: 'File' };
  return { id: `${kind}-${++tabSeq}`, kind, title: titles[kind], ...patch };
}

export function WorkspacePanel({ busy, revision, visible = true, onReveal, onChooseWorkspace, reviewRequest = 0, host }: { host?: HTMLElement | null; onChooseWorkspace?: (path: string) => Promise<string>; busy: boolean; revision: string; visible?: boolean; onReveal?: () => void; reviewRequest?: number }) {
  const [open, setOpen] = useState(false);
  const [size, setSize] = useState(45);
  const [resizing, setResizing] = useState(false);
  const [stacked, setStacked] = useState(false);
  const resizePointer = useRef<number | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [root, setRoot] = useState('');
  const [error, setError] = useState('');
  const [mutating, setMutating] = useState(false);
  const [fileRevision, setFileRevision] = useState(0);
  const [dialog, setDialog] = useState<{ id: number; options: DialogOptions; resolve: (value: DialogResult) => void } | null>(null);
  const dialogId = useRef(0);
  const handles = useRef(new Map<string, EditorHandle>());
  const dialogPending = useRef(false);
  const tabsRef = useRef(tabs);
  tabsRef.current = tabs;
  const active = tabs.find(t => t.id === activeId) ?? null;
  const treeRoot = active?.kind === 'file' ? active.root ?? root : root;
  const fileContext = active?.kind === 'files' || active?.kind === 'file';
  const dirty = tabs.some(tab => tab.dirty);

  const requestDialog = useCallback((options: DialogOptions): Promise<DialogResult> => {
    if (dialogPending.current) return Promise.resolve({ action: 'cancel', value: '' });
    dialogPending.current = true;
    return new Promise(resolve => setDialog({ id: ++dialogId.current, options, resolve }));
  }, []);

  function updateTab(id: string, patch: Partial<WorkspaceTab>) {
    setTabs(current => current.map(tab => tab.id === id && Object.entries(patch).some(([key, value]) => tab[key as keyof WorkspaceTab] !== value) ? { ...tab, ...patch } : tab));
  }

  function openKind(kind: TabKind, fresh: boolean) {
    if (dialogPending.current) return;
    setError(''); setOpen(true);
    const existing = !fresh || SINGLETONS.has(kind) ? tabs.find(t => t.kind === kind) : undefined;
    if (existing) { setActiveId(existing.id); return; }
    const tab = makeTab(kind, kind === 'terminal' ? { root } : {});
    setTabs(current => [...current, tab]); setActiveId(tab.id);
  }

  function openFile(path: string, fileRoot = treeRoot) {
    setError(''); setOpen(true);
    const existing = tabs.find(tab => tab.kind === 'file' && tab.path === path && tab.root === fileRoot);
    if (existing) { setActiveId(existing.id); return; }
    const tab = makeTab('file', { title: path.split('/').pop() ?? path, path, root: fileRoot });
    setTabs(current => [...current, tab]); setActiveId(tab.id);
  }
  useEffect(() => { if (reviewRequest) openKind('review', false); }, [reviewRequest]);

  async function resolveEdits(affected: WorkspaceTab[]) {
    if (affected.some(tab => handles.current.get(tab.id)?.saving())) {
      setError('Wait for the file to finish saving.'); return false;
    }
    const modified = affected.filter(tab => handles.current.get(tab.id)?.dirty());
    if (!modified.length) return true;
    const result = await requestDialog({
      title: 'Save your changes?', message: `${modified.map(tab => tab.path).join(', ')} ${modified.length === 1 ? 'has' : 'have'} unsaved changes.`,
      confirm: modified.length === 1 ? 'Save' : 'Save all', alternate: 'Discard changes',
    });
    if (result.action === 'cancel') return false;
    if (result.action === 'alternate') return true;
    for (const tab of modified) {
      if (!await handles.current.get(tab.id)?.save()) {
        setActiveId(tab.id); setOpen(true); onReveal?.(); setError(`Could not save ${tab.path}. Your edits are still open.`); return false;
      }
    }
    return true;
  }

  function removeTabs(removed: WorkspaceTab[]) {
    const ids = new Set(removed.map(tab => tab.id));
    removed.forEach(tab => {
      if (tab.kind === 'terminal') closeTerminalForTab(tab.id);
      if (tab.kind === 'browser') void closeBrowserForTab(tab.id).catch(e => setError(errorMessage(e)));
    });
    const currentTabs = tabsRef.current;
    setTabs(current => current.filter(tab => !ids.has(tab.id)));
    setActiveId(current => {
      if (!current || !ids.has(current)) return current;
      const remaining = currentTabs.filter(tab => !ids.has(tab.id));
      const index = currentTabs.findIndex(tab => tab.id === current);
      return remaining[Math.min(Math.max(0, index - 1), remaining.length - 1)]?.id ?? null;
    });
  }

  async function closeTab(tab: WorkspaceTab) {
    if (dialogPending.current || mutating) return;
    if (await resolveEdits([tab])) removeTabs([tab]);
  }

  async function choose() {
    if (!nativeAvailable || mutating || dialogPending.current) return;
    try {
      const selected = await openDialog({ directory: true, title: 'Choose workspace' });
      if (typeof selected !== 'string' || selected === root) return;
      const files = tabs.filter(tab => tab.kind === 'file');
      if (!await resolveEdits(files)) return;
      const nextRoot = onChooseWorkspace ? await onChooseWorkspace(selected) : (await api.setWorkspace(selected), selected);
      removeTabs(files); setRoot(nextRoot); setError(''); setFileRevision(n => n + 1);
    } catch (e) { setError(errorMessage(e)); }
  }

  async function treeAction(action: TreeAction, path: string, directory: boolean) {
    if (mutating || dialogPending.current) return;
    const issuedRoot = treeRoot;
    const affected = tabs.filter(tab => tab.kind === 'file' && tab.root === issuedRoot && (tab.path === path || (directory && tab.path?.startsWith(`${path}/`))));
    let mutation: EditorMutation;
    if (action === 'delete') {
      if (affected.some(tab => handles.current.get(tab.id)?.saving())) { setError('Wait for the file to finish saving.'); return; }
      const result = await requestDialog({ title: `Delete ${directory ? 'folder' : 'file'}?`, message: `Permanently delete “${path}”${directory ? ' and all its contents' : ''}?${affected.some(tab => tab.dirty) ? ' Unsaved edits in this entry will also be discarded.' : ''} This cannot be undone.`, confirm: 'Delete', destructive: true });
      if (result.action !== 'confirm') return;
      mutation = { kind: 'delete', path, directory, recursive: directory };
    } else {
      const creating = action !== 'rename';
      const result = await requestDialog({
        title: creating ? action === 'createFile' ? 'New file' : 'New folder' : 'Rename',
        message: creating ? `Create in ${path || 'workspace root'}.` : `Rename “${path}”.`,
        input: creating ? '' : path.split('/').pop(), inputLabel: creating ? 'Name or relative path' : 'New name', confirm: creating ? 'Create' : 'Rename',
      });
      if (result.action !== 'confirm') return;
      mutation = action === 'rename' ? { kind: 'rename', path, name: result.value } : { kind: action, path: path ? `${path}/${result.value}` : result.value };
      if (action === 'rename' && !await resolveEdits(affected)) return;
    }
    setMutating(true); setError('');
    try {
      const nextPath = await api.editorMutate(issuedRoot, mutation);
      if (mutation.kind === 'delete') removeTabs(affected);
      if (mutation.kind === 'rename') setTabs(current => current.map(tab => affected.some(item => item.id === tab.id)
        ? { ...tab, path: nextPath + tab.path!.slice(path.length), title: (nextPath + tab.path!.slice(path.length)).split('/').pop()!, dirty: false } : tab));
      setFileRevision(n => n + 1);
      if (mutation.kind === 'createFile') openFile(nextPath, issuedRoot);
    } catch (e) { setError(errorMessage(e)); }
    finally { setMutating(false); }
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!visible || event.defaultPrevented || dialogPending.current || (event.target as HTMLElement | null)?.closest?.('.terminal-host')) return;
      if (event.key === 'Escape' && (event.target as HTMLElement | null)?.closest?.('.cm-editor')) return;
      if (event.key === 'Escape') { if (expanded) setExpanded(false); else setOpen(false); return; }
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      const kind: TabKind | null = key === 'g' && event.shiftKey ? 'review' : !event.shiftKey && key === '`' ? 'terminal' : !event.shiftKey && key === 'p' ? 'files' : !event.shiftKey && key === 't' ? 'browser' : null;
      if (kind) { event.preventDefault(); openKind(kind, false); }
      if (key === 's' && open && active?.kind === 'file') { event.preventDefault(); void handles.current.get(active.id)?.save(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    const sync = () => api.getWorkspace().then(w => { if (!disposed) setRoot(w.path); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    void sync();
    window.addEventListener('focus', sync);
    return () => { disposed = true; window.removeEventListener('focus', sync); };
  }, [revision]);

  useEffect(() => {
    if (!dirty) return;
    const onUnload = (event: BeforeUnloadEvent) => { event.preventDefault(); event.returnValue = ''; };
    window.addEventListener('beforeunload', onUnload);
    let disposed = false;
    let stop: (() => void) | undefined;
    if (nativeAvailable) void getCurrentWindow().onCloseRequested(async event => {
      event.preventDefault();
      if (await resolveEdits(tabsRef.current)) await getCurrentWindow().destroy();
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(e => setError(errorMessage(e)));
    return () => { disposed = true; stop?.(); window.removeEventListener('beforeunload', onUnload); };
    // resolveEdits reads current handles; tab changes must not introduce a gap
    // in the native close handler.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dirty]);

  useEffect(() => {
    const split = host?.parentElement;
    if (!split) return;
    const sync = () => setStacked(split.getBoundingClientRect().width < 680);
    sync();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(sync);
    observer.observe(split);
    return () => observer.disconnect();
  }, [host]);

  // Keep the chosen split when hiding, maximizing, or switching conversations.
  const clampSize = (value: number) => Math.min(70, Math.max(25, value));
  const panel = <aside hidden={!open || !visible} style={{ '--workspace-panel-size': `${size}%` } as React.CSSProperties} className={`workspace-panel ${expanded ? 'panel-expanded' : ''} ${resizing ? 'panel-resizing' : ''}`} data-native-browser={nativeAvailable && active?.kind === 'browser' || undefined} aria-label="Workspace inspector">
      {!expanded && <div className="workspace-resize-handle" role="separator" tabIndex={0} aria-label="Resize workspace panel" aria-orientation={stacked ? 'horizontal' : 'vertical'} aria-valuemin={25} aria-valuemax={70} aria-valuenow={size} aria-valuetext={`${Math.round(size)} percent`} title="Drag to resize · Arrow keys to adjust" onDoubleClick={() => setSize(45)} onKeyDown={event => {
        const increase = stacked ? 'ArrowUp' : 'ArrowLeft';
        const decrease = stacked ? 'ArrowDown' : 'ArrowRight';
        if ([increase, decrease, 'Home', 'End'].includes(event.key)) {
          event.preventDefault();
          setSize(current => event.key === 'Home' ? 25 : event.key === 'End' ? 70 : clampSize(current + (event.key === increase ? 2 : -2)));
        }
      }} onPointerDown={event => {
        if (event.button !== 0) return;
        event.preventDefault();
        resizePointer.current = event.pointerId;
        event.currentTarget.setPointerCapture(event.pointerId);
        setResizing(true);
      }} onPointerMove={event => {
        if (resizePointer.current !== event.pointerId) return;
        const bounds = (host?.parentElement ?? event.currentTarget.parentElement?.parentElement)?.getBoundingClientRect();
        if (!bounds) return;
        const length = stacked ? bounds.height : bounds.width;
        if (length > 0) setSize(clampSize(100 * (stacked ? bounds.bottom - event.clientY : bounds.right - event.clientX) / length));
      }} onPointerUp={event => {
        resizePointer.current = null;
        setResizing(false);
        event.currentTarget.releasePointerCapture(event.pointerId);
      }} onLostPointerCapture={() => { resizePointer.current = null; setResizing(false); }} onPointerCancel={() => { resizePointer.current = null; setResizing(false); }} />}
      <div className="workspace-tabs">
        <div className="workspace-tab-list" role="tablist" aria-label="Workspace tabs">
          {!tabs.length && <span className="workspace-panel-title">Workspace</span>}
          {tabs.map(tab => {
            const Icon = TAB_ICONS[tab.kind];
            return <div key={tab.id} className={`workspace-tab ${tab.id === activeId ? 'active' : ''}`}>
              <button role="tab" tabIndex={tab.id === activeId ? 0 : -1} aria-selected={tab.id === activeId} id={`tab-${tab.id}`} aria-controls={`workspace-content-${tab.id}`} onClick={() => setActiveId(tab.id)} onKeyDown={event => {
                const index = tabs.findIndex(item => item.id === tab.id);
                const next = event.key === 'ArrowRight' ? (index + 1) % tabs.length : event.key === 'ArrowLeft' ? (index - 1 + tabs.length) % tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : -1;
                if (next >= 0) { event.preventDefault(); setActiveId(tabs[next].id); document.getElementById(`tab-${tabs[next].id}`)?.focus(); }
              }} title={tab.path ? `${tab.root}/${tab.path}` : tab.title}>
                <Icon size={13} className="workspace-tab-icon" /><span className="workspace-tab-title">{tab.title}</span>{tab.dirty && <span className="tab-dirty" aria-label="Unsaved changes">●</span>}
              </button>
              <button className="workspace-tab-close" aria-label={`Close ${tab.title}`} onClick={() => void closeTab(tab)}><X size={12} /></button>
            </div>;
          })}
        </div>
        <details className="app-menu workspace-add" name="application-menu">
          <summary className="icon-button" aria-label="New workspace tab" title="New workspace tab"><Plus size={16} /></summary>
          <nav className="menu" aria-label="Workspace shortcuts">{LAUNCHERS.map(item => <button key={item.kind} onClick={event => { event.currentTarget.closest('details')?.removeAttribute('open'); openKind(item.kind, true); }}><item.icon size={15} /><span>{item.label}</span><kbd>{item.shortcut}</kbd></button>)}</nav>
        </details>
        <div className="workspace-tabs-spacer" />
        <div className="workspace-tab-actions">
          <button className="icon-button" title={expanded ? 'Restore panel size' : 'Expand panel'} aria-label={expanded ? 'Restore panel size' : 'Expand panel'} aria-pressed={expanded} onClick={() => setExpanded(!expanded)}>{expanded ? <Minimize2 size={16} /> : <Maximize2 size={16} />}</button>
          <button className="icon-button selected" aria-label="Close inspector" onClick={() => { setOpen(false); setExpanded(false); }}><PanelRight size={16} /></button>
        </div>
      </div>
      {error && <div className="workspace-error" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss workspace error" onClick={() => setError('')}><X size={14} /></button></div>}
      <div className="workspace-body">
        <WorkspaceTree root={treeRoot} hidden={!open || !visible || !fileContext} revision={fileRevision} selectedPath={active?.path} disabled={mutating || busy || !nativeAvailable} onOpenFile={openFile} onChooseWorkspace={() => void choose()} onAction={(...args) => void treeAction(...args)} onError={setError} />
        <div className="workspace-tab-content">
          {tabs.length === 0 && <nav className="workspace-empty" aria-label="Workspace shortcuts"><h2>Tools</h2>{LAUNCHERS.map(item => <button key={item.kind} onClick={() => openKind(item.kind, false)}><item.icon size={17} /><span>{item.label}</span><kbd>{item.shortcut}</kbd></button>)}</nav>}
          {active && active.kind !== 'file' && <div className="workspace-feature-content" role="tabpanel" id={`workspace-content-${active.id}`} aria-labelledby={`tab-${active.id}`}>
          {open && visible && active?.kind === 'terminal' && <TerminalPanel key={active.id} tabKey={active.id} root={active.root} />}
          {open && visible && active?.kind === 'browser' && <BrowserPanel key={active.id} tabId={active.id} onError={setError} onNavigate={url => {
            let title = 'New tab'; try { title = new URL(url).hostname || title; } catch { /* keep */ }
            updateTab(active.id, { title });
          }} />}
          {open && visible && active?.kind === 'review' && <ReviewPanel root={root} revision={`${revision}:${fileRevision}`} busy={busy} onChoose={() => void choose()} onOpenFile={path => openFile(path, root)} />}
          {active?.kind === 'files' && <div className="file-open-empty"><FolderOpen size={32} /><h2>Editor</h2><p>{nativeAvailable ? root ? 'Select a file in the explorer, or create something new.' : 'Choose a workspace to start editing.' : 'Open the desktop application to edit local files.'}</p><div className="editor-empty-actions">{root ? <><button className="file-open" onClick={() => void treeAction('createFile', '', false)}>New file</button><button className="file-open" onClick={() => void treeAction('createDirectory', '', true)}>New folder</button></> : <button className="file-open" disabled={!nativeAvailable || busy} onClick={() => void choose()}>Choose workspace</button>}</div><p className="muted">Ctrl+S save · Ctrl+F find · Ctrl+Z undo</p></div>}
          </div>}
          {tabs.filter(tab => tab.kind === 'file' && tab.path && tab.root).map(tab => <div key={tab.id} className="editor-tab-content" hidden={!open || !visible || activeId !== tab.id} id={`workspace-content-${tab.id}`} role="tabpanel" aria-labelledby={`tab-${tab.id}`}>
            <FileEditor root={tab.root!} path={tab.path!} active={open && visible && activeId === tab.id} onDirty={value => updateTab(tab.id, { dirty: value })} onSaved={() => setFileRevision(n => n + 1)} register={handle => { if (handle) handles.current.set(tab.id, handle); else handles.current.delete(tab.id); }} requestDialog={requestDialog} />
          </div>)}
        </div>
      </div>
    </aside>;

  return <div className="workspace-panel-container">
    <div className="panel-toggles" hidden={!visible || open}>
      <button className="icon-button" title="Workspace panel" aria-label="Toggle workspace panel" aria-expanded={open} onClick={() => { setOpen(!open); setExpanded(false); }}><PanelRight size={17} /></button>
    </div>
    {host ? createPortal(panel, host) : panel}

    {dialog && <WorkspaceDialog key={dialog.id} options={dialog.options} onResult={result => { dialogPending.current = false; setDialog(null); dialog.resolve(result); }} />}
  </div>;
}
