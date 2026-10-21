import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Copy, ExternalLink, RotateCw, Save } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { DialogOptions, DialogResult } from './WorkspaceDialog';

const CodeEditor = lazy(() => import('./CodeEditor').then(module => ({ default: module.CodeEditor })));

export interface EditorHandle { save: () => Promise<boolean>; dirty: () => boolean; saving: () => boolean }
export function FileEditor({ root, path, active, onDirty, onSaved, register, requestDialog }: {
  root: string; path: string; active: boolean;
  onDirty: (dirty: boolean) => void; onSaved: () => void;
  register: (handle: EditorHandle | null) => void;
  requestDialog: (options: DialogOptions) => Promise<DialogResult>;
}) {
  const [loaded, setLoaded] = useState<{ content: string; sha256: string; ending: string; key: string } | null>(null);
  const [content, setContent] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [reload, setReload] = useState(0);
  const [cursor, setCursor] = useState({ line: 1, column: 1 });
  const current = useRef({ loaded, content, saving });
  current.current = { loaded, content, saving };
  const dirty = loaded !== null && content !== loaded.content;
  const savePromise = useRef<Promise<boolean> | null>(null);
  const mounted = useRef(true);
  const onDirtyRef = useRef(onDirty);
  onDirtyRef.current = onDirty;

  useEffect(() => {
    let cancelled = false;
    setLoaded(null); setError(''); setNotice('');
    api.editorRead(root, path).then(doc => {
      if (cancelled) return;
      const normalized = doc.content.replace(/\r\n/g, '\n');
      setContent(normalized);
      setLoaded({ content: normalized, sha256: doc.sha256, ending: doc.content.includes('\r\n') ? '\r\n' : '\n', key: `${root}:${path}:${reload}` });
      setCursor({ line: 1, column: 1 });
    }).catch(e => { if (!cancelled) setError(errorMessage(e)); });
    return () => { cancelled = true; };
  }, [root, path, reload]);

  useEffect(() => { onDirtyRef.current(dirty); }, [dirty]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  function save(): Promise<boolean> {
    if (savePromise.current) return savePromise.current;
    const snapshot = current.current;
    if (!snapshot.loaded || snapshot.loaded.content === snapshot.content) return Promise.resolve(!!snapshot.loaded);
    setSaving(true); setError(''); setNotice('');
    const pending = api.editorSave(root, path, snapshot.content.replace(/\n/g, snapshot.loaded.ending), snapshot.loaded.sha256)
      .then(doc => {
        if (mounted.current) {
          setLoaded({ ...snapshot.loaded!, content: snapshot.content, sha256: doc.sha256 });
          setNotice('Saved'); onSaved();
        }
        // Typing can continue during a save; closing requires every edit saved.
        return current.current.content === snapshot.content;
      })
      .catch(e => { if (mounted.current) setError(errorMessage(e)); return false; })
      .finally(() => { savePromise.current = null; if (mounted.current) setSaving(false); });
    savePromise.current = pending;
    return pending;
  }
  const saveRef = useRef(save);
  saveRef.current = save;
  useEffect(() => {
    register({ save: () => saveRef.current(), dirty: () => !!current.current.loaded && current.current.content !== current.current.loaded.content, saving: () => savePromise.current !== null });
    return () => register(null);
  }, [register]);

  async function reloadFile() {
    if (saving) return;
    if (dirty) {
      const result = await requestDialog({ title: 'Reload file?', message: `Reloading ${path} will discard your unsaved edits.`, confirm: 'Discard & reload', destructive: true });
      if (result.action !== 'confirm') return;
    }
    setReload(n => n + 1);
  }

  return <section className="file-view" hidden={!active} aria-label={`Editor ${path}`}>
    <div className="file-bar">
      <span className="file-breadcrumb" title={`${root}/${path}`}><strong>{root.split(/[\\/]/).filter(Boolean).pop()}</strong> › {path}</span>
      <div className="file-bar-actions">
        <button className="icon-button" aria-label="Copy file contents" disabled={!loaded} onClick={() => void navigator.clipboard.writeText(content).then(() => setNotice('Copied')).catch(e => setError(errorMessage(e)))}><Copy size={15} /></button>
        <button className="icon-button" aria-label="Open in external editor" onClick={() => void api.openWorkspacePath(`${root}/${path}`).catch(e => setError(errorMessage(e)))}><ExternalLink size={15} /></button>
        <button className="icon-button" aria-label="Reload file" disabled={saving} onClick={() => void reloadFile()}><RotateCw size={15} /></button>
        <button className="file-open" title="Save (Ctrl+S)" disabled={!dirty || saving} onClick={() => void save()}><Save size={14} />{saving ? 'Saving…' : 'Save'}</button>
      </div>
    </div>
    {error && <div className="editor-error" role="alert">{error}<button onClick={() => void reloadFile()} disabled={saving}>Reload from disk</button></div>}
    {!loaded && !error && <p className="tree-loading" role="status">Opening file…</p>}
    {loaded && <Suspense fallback={<p role="status">Loading editor…</p>}><CodeEditor key={loaded.key} initialContent={loaded.content} path={path} onChange={value => { setContent(value); setNotice(''); }} onSave={() => void saveRef.current()} onCursor={(line, column) => setCursor({ line, column })} /></Suspense>}
    {loaded && <footer className="editor-status"><span role="status">{dirty ? 'Unsaved changes' : notice || 'Saved'}</span><span>Ln {cursor.line}, Col {cursor.column}</span><span>UTF-8 · {loaded.ending === '\r\n' ? 'CRLF' : 'LF'}</span><span>Ctrl+S save · Ctrl+F find</span></footer>}
  </section>;
}
