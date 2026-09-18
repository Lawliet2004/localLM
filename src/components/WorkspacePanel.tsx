import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { openPath, openUrl } from '@tauri-apps/plugin-opener';
import { File, Folder, FolderOpen, GitBranch, PanelRight, PanelBottom, Terminal, RefreshCw, ArrowLeft, Maximize2, Minimize2, Globe, Files, ListTodo } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { CommandRunCard } from './CommandRunCard';
import type { ArtifactRecord } from '../lib/types';

export function WorkspacePanel({ busy, conversationId, revision, onBusy }: { busy: boolean; conversationId: string | null; revision: string; onBusy: (busy: boolean) => void }) {
  const [panel, setPanel] = useState<'home' | 'files' | 'terminal' | 'changes' | 'artifacts' | 'browser' | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [browserUrl, setBrowserUrl] = useState('');
  const launchers = [
    { id: 'changes', label: 'Review', icon: GitBranch, shortcut: 'Ctrl+Shift+G' },
    { id: 'terminal', label: 'Terminal', icon: Terminal, shortcut: 'Ctrl+`' },
    { id: 'browser', label: 'Browser', icon: Globe, shortcut: 'Ctrl+T' },
    { id: 'files', label: 'Files', icon: Files, shortcut: 'Ctrl+P' },
    { id: 'artifacts', label: 'Artifacts', icon: File, shortcut: '' },
  ] as const;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey)) return;
      const key = event.key.toLowerCase();
      const target = key === 'g' && event.shiftKey ? 'changes' : !event.shiftKey && key === '`' ? 'terminal' : !event.shiftKey && key === 'p' ? 'files' : !event.shiftKey && key === 't' ? 'browser' : null;
      if (target) { event.preventDefault(); setPanel(target); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const [root, setRoot] = useState('');
  const [path, setPath] = useState('.');
  const [entries, setEntries] = useState<{name: string; kind: string}[]>([]);
  const [preview, setPreview] = useState('');
  const [git, setGit] = useState<{branch: string; branches: string[]; status: string; diff: string} | null>(null);
  const [artifacts, setArtifacts] = useState<ArtifactRecord[]>([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [command, setCommand] = useState('');
  const [run, setRun] = useState<{command: string; stdout: string; stderr: string; status: 'running' | 'completed' | 'failed' | 'interrupted'; exitCode?: number | null; durationMs?: number}>();
  async function browse(next: string) { const data = await api.workspaceInspect(next, true); setPath(next); setEntries(data.entries ?? []); setPreview(data.truncated ? 'First 500 entries shown.' : ''); }
  async function act(action: () => Promise<unknown>) { setLoading(true); setError(''); try { await action(); } catch (e) { setError(errorMessage(e)); } finally { setLoading(false); } }
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getWorkspace().then(w => { if (!disposed) { setRoot(w.path); setPath('.'); setEntries([]); setPreview(''); setGit(null); } }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [revision]);
  useEffect(() => {
    if (!nativeAvailable || !panel) return;
    void act(async () => {
      if (panel === 'files' && root) await browse('.');
      if (panel === 'changes' && root) setGit(await api.workspaceGit());
      if (panel === 'artifacts' && conversationId) setArtifacts(await api.listConversationArtifacts(conversationId));
    });
  }, [panel, root, conversationId]);
  useEffect(() => { document.documentElement.classList.toggle('workspace-panel-open', Boolean(panel)); return () => document.documentElement.classList.remove('workspace-panel-open'); }, [panel]);
  async function choose() {
    const selected = await open({ directory: true, title: 'Choose workspace' });
    if (typeof selected === 'string') { await api.setWorkspace(selected); setRoot(selected); setPanel('files'); }
  }
  async function execute() {
    if (!command.trim() || busy || run?.status === 'running') return;
    const text = command; setCommand(''); setError(''); onBusy(true);
    setRun({ command: text, stdout: '', stderr: '', status: 'running' });
    try {
      const result = await api.workspaceCommand(text, e => setRun(current => current ? { ...current, [e.stream === 'stderr' ? 'stderr' : 'stdout']: (current[e.stream === 'stderr' ? 'stderr' : 'stdout'] + e.chunk).slice(-131072) } : current));
      setRun({ command: text, ...result, status: result.exitCode === 0 ? 'completed' : 'failed' });
    } catch (e) { setError(errorMessage(e)); setRun(current => current ? { ...current, status: 'interrupted' } : current); } finally { onBusy(false); }
  }
  return <>
    <div className="panel-toggles">
      <button className="icon-button" title="Review changes" aria-label="Git branches and changes" aria-pressed={panel === 'changes'} onClick={() => setPanel(panel === 'changes' ? null : 'changes')}><ListTodo size={18} /></button>
      <button className="icon-button" title="Terminal" aria-label="Toggle terminal" aria-pressed={panel === 'terminal'} onClick={() => setPanel(panel === 'terminal' ? null : 'terminal')}><PanelBottom size={17} /></button>
      <button className="icon-button" title="Right panel" aria-label="Toggle files panel" aria-expanded={Boolean(panel)} onClick={() => setPanel(panel ? null : 'home')}><PanelRight size={17} /></button>
    </div>
    {panel && <aside className={`workspace-panel ${expanded ? 'panel-expanded' : ''} ${panel === 'home' ? 'panel-home' : ''}`} aria-label="Workspace inspector">
      <header className="inspector-header">
        {panel !== 'home' && <button className="icon-button" title="Panel shortcuts" aria-label="Panel shortcuts" onClick={() => setPanel('home')}><ArrowLeft size={16} /></button>}
        <span>{panel === 'home' ? '' : launchers.find(item => item.id === panel)?.label}</span>
        <div className="inspector-actions">
          <button className="icon-button" aria-label={expanded ? 'Restore panel size' : 'Expand panel'} onClick={() => setExpanded(!expanded)}>{expanded ? <Minimize2 size={17} /> : <Maximize2 size={17} />}</button>
          <button className="icon-button" aria-label="Open terminal in panel" onClick={() => setPanel('terminal')}><PanelBottom size={17} /></button>
          <button className="icon-button selected" aria-label="Close inspector" onClick={() => setPanel(null)}><PanelRight size={17} /></button>
        </div>
      </header>
      {panel === 'home' && <nav className="panel-launcher" aria-label="Workspace shortcuts">{launchers.map(item => <button key={item.id} onClick={() => { setPanel(item.id); setError(''); }}><item.icon size={17} /><span>{item.label}</span>{item.shortcut && <kbd>{item.shortcut}</kbd>}</button>)}</nav>}
      {error && <p className="error-banner" role="alert">{error}</p>}
      {loading && <p role="status">Loading…</p>}
      {panel !== 'home' && panel !== 'browser' && <div className="panel-workspace-actions"><button className="project-chip" disabled={busy || !nativeAvailable} title={root || 'Choose workspace'} onClick={() => void act(choose)}><Folder size={15} />{root ? root.split(/[\\/]/).pop() : 'Choose workspace'}</button><button className="icon-button" aria-label="Open workspace folder" disabled={!root || !nativeAvailable} onClick={() => void act(() => openPath(root))}><FolderOpen size={17} /></button></div>}
      {panel === 'browser' && <form className="panel-browser" onSubmit={event => { event.preventDefault(); void act(async () => { const url = new URL(browserUrl); if (!['https:', 'http:'].includes(url.protocol)) throw new Error('Enter an HTTP or HTTPS address.'); if (nativeAvailable) await openUrl(url.href); else window.open(url.href, '_blank', 'noopener,noreferrer'); }); }}><Globe size={28} /><h2>Open a website</h2><p className="muted">Open a link in your default browser.</p><label htmlFor="panel-browser-url">Website address</label><input id="panel-browser-url" type="url" placeholder="https://example.com" value={browserUrl} onChange={event => setBrowserUrl(event.target.value)} required /><button className="primary" disabled={loading}>Open in browser</button></form>}
      {panel === 'changes' && !root && <p className="panel-empty">Choose a workspace to review its changes.</p>}
      {panel === 'files' && <>
        <div className="panel-toolbar"><button disabled={path === '.' || loading} aria-label="Parent folder" onClick={() => void act(() => browse(path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '.'))}><ArrowLeft size={15} /></button><span>{path}</span><button aria-label="Refresh files" disabled={loading || !root} onClick={() => void act(() => browse(path))}><RefreshCw size={15} /></button></div>
        {!root && <p>Choose a workspace folder to browse files.</p>}
        <div className="file-list">{entries.map(entry => <button key={entry.name} disabled={entry.kind === 'link' || loading} onClick={() => void act(async () => {
          const target = path === '.' ? entry.name : `${path}/${entry.name}`;
          if (entry.kind === 'directory') await browse(target); else { const result = await api.workspaceInspect(target, false); setPreview(`${target}\n\n${result.content ?? ''}`); }
        })}>{entry.kind === 'directory' ? <Folder size={15} /> : <File size={15} />}{entry.name}</button>)}</div>
        {preview && <pre className="file-preview">{preview}</pre>}
      </>}
      {panel === 'changes' && git && <><label className="panel-toolbar">Branch<select aria-label="Switch branch" value={git.branch} disabled={busy || loading} onChange={e => void act(async () => setGit(await api.workspaceGit(e.target.value)))}>{!git.branch && <option value="">Detached HEAD</option>}{git.branches.map(b => <option key={b}>{b}</option>)}</select></label><pre>{git.status || 'Working tree clean'}</pre><pre>{git.diff || 'No unstaged diff.'}</pre></>}
      {panel === 'artifacts' && <>{!artifacts.length && <p>No saved artifacts in this task.</p>}{artifacts.map(a => <details key={a.id}><summary>{a.toolName} · {Math.ceil(a.sizeBytes/1024)} KB</summary><pre>{a.content}</pre></details>)}</>}
      {panel === 'terminal' && <><p className="muted">Commands run in {root || 'the selected workspace'}.</p>
        {run && <CommandRunCard toolName="execute_command" {...run} cwd={root} />}
        <form className="terminal-form" onSubmit={e => { e.preventDefault(); void execute(); }}><textarea aria-label="Terminal command" placeholder="Enter a command…" rows={3} value={command} onChange={e => setCommand(e.target.value)} disabled={!root || busy} />
          {run?.status === 'running' ? <button type="button" onClick={() => void act(() => api.cancelGeneration())}>Stop command</button> : <button className="primary" disabled={!root || busy || !command.trim() || !nativeAvailable}>Run command</button>}
        </form></>}
    </aside>}
  </>;
}
