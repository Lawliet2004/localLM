import { useEffect, useRef, useState } from 'react';
import { FileCode, FolderOpen, GitBranch, RefreshCw } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { confirm } from '@tauri-apps/plugin-dialog';
import { DiffLines } from './DiffView';
import type { GitAction, GitHistoryEntry } from '../lib/types';

export function ReviewPanel({ root, revision, busy, onChoose, onOpenFile }: {
  root: string; revision: string; busy: boolean; onChoose: () => void;
  onOpenFile: (path: string) => void;
}) {
  const [git, setGit] = useState<Awaited<ReturnType<typeof api.workspaceGit>> | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [commitMessage, setCommitMessage] = useState('');
  const [branchName, setBranchName] = useState('');
  const [history, setHistory] = useState<GitHistoryEntry[] | null>(null);
  const [notice, setNotice] = useState('');
  const rootRef = useRef(root);
  rootRef.current = root;
  useEffect(() => {
    if (!root || !nativeAvailable) return;
    let disposed = false;
    setLoading(true); setError(''); setGit(null); setHistory(null); setNotice('');
    api.workspaceGit(undefined, root).then(value => { if (!disposed) setGit(value); })
      .catch(e => { if (!disposed) setError(errorMessage(e)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [root, revision, refresh]);
  useEffect(() => {
    const update = () => setRefresh(n => n + 1);
    window.addEventListener('focus', update);
    return () => window.removeEventListener('focus', update);
  }, []);

  async function switchBranch(branch: string) {
    const issuedRoot = root;
    setLoading(true); setError('');
    try { const result = await api.workspaceGit(branch, issuedRoot); if (rootRef.current === issuedRoot) setGit(result); }
    catch (e) { if (rootRef.current === issuedRoot) setError(errorMessage(e)); }
    finally { if (rootRef.current === issuedRoot) setLoading(false); }
  }
  async function gitAction(request: GitAction) {
    const issuedRoot = root;
    setLoading(true); setError(''); setNotice('');
    try {
      if (request.action === 'history') {
        const result = await api.workspaceGitAction<{ entries: GitHistoryEntry[] }>(issuedRoot, request);
        if (rootRef.current === issuedRoot) setHistory(result.entries);
      } else {
        const result = await api.workspaceGitAction(issuedRoot, request);
        if (rootRef.current === issuedRoot) {
          setGit(result);
          if (request.action === 'commit') { setCommitMessage(''); setNotice('Commit created.'); setHistory(null); }
          if (request.action === 'createBranch') { setBranchName(''); setNotice(`Branch ${request.name} created. Choose it above to switch.`); }
        }
      }
    } catch (e) { if (rootRef.current === issuedRoot) setError(errorMessage(e)); }
    finally { if (rootRef.current === issuedRoot) setLoading(false); }
  }
  async function commit() {
    if (!git?.stagedTree || !commitMessage.trim()) return;
    const issuedRoot = root;
    try {
      const approved = await confirm(`Create a commit in ${root} with the currently staged changes?\n\n${commitMessage}`, { title: 'Confirm commit', kind: 'warning' });
      if (approved && rootRef.current === issuedRoot) await gitAction({ action: 'commit', message: commitMessage, expectedTree: git.stagedTree, confirmed: true });
    } catch (e) { if (rootRef.current === issuedRoot) setError(errorMessage(e)); }
  }
  return <div className="review-view">
    <div className="review-toolbar">
      <button className="file-open" title={root} disabled={busy || !nativeAvailable} onClick={onChoose}><FolderOpen size={15} />{root.split(/[\\/]/).filter(Boolean).pop() || 'Choose workspace'}</button>
      <span className="review-heading">Changes</span>
      <button className="icon-button" aria-label="Refresh changes" disabled={!root || loading || !nativeAvailable} onClick={() => setRefresh(n => n + 1)}><RefreshCw size={15} /></button>
    </div>
    {!nativeAvailable ? <p className="panel-empty">Open the desktop application to review local Git changes.</p> : !root ? <p className="panel-empty">Choose a workspace to review its changes.</p> : null}
    {loading && <p role="status">Loading changes…</p>}
    {error && <p className="editor-error" role="alert">{error}<button onClick={() => setRefresh(n => n + 1)}>Retry</button></p>}
    {notice && <p role="status">{notice}</p>}
    {git?.repository === false && <div className="file-open-empty"><GitBranch size={28} /><h2>No Git repository</h2><p>This folder can still be edited. Select a Git repository to review changes.</p></div>}
    {git && git.repository !== false && <>
      <label className="panel-toolbar"><GitBranch size={15} />Branch<select aria-label="Switch branch" value={git.branch} disabled={busy || loading} onChange={e => void switchBranch(e.target.value)}>{!git.branch && <option value="">{git.branches.length ? 'Detached HEAD' : 'No commits yet'}</option>}{git.branches.map(branch => <option key={branch}>{branch}</option>)}</select></label>
      <div className="review-files">
        {git.files?.map(file => <div className="review-file-row" key={file.path}><button onClick={() => onOpenFile(file.path)} disabled={file.status.trim() === 'D'} title={`Open ${file.path} in Editor`}><span className="review-file-status" title="Index / working tree status">{file.status}</span><FileCode size={14} /><span>{file.previousPath ? `${file.previousPath} → ` : ''}{file.path}</span></button>
          {file.status !== '??' && file.status[0] !== ' ' && <button className="secondary" aria-label={`Unstage ${file.path}`} disabled={busy || loading} onClick={() => void gitAction({ action: 'unstage', paths: [file.path, ...(file.previousPath ? [file.previousPath] : [])] })}>Unstage</button>}
          {(file.status === '??' || file.status[1] !== ' ') && <button className="secondary" aria-label={`Stage ${file.path}`} disabled={busy || loading} onClick={() => void gitAction({ action: 'stage', paths: [file.path] })}>Stage</button>}
        </div>)}
        {!git.files?.length && <pre>{git.status || 'Working tree clean'}</pre>}
      </div>
      <form className="review-commit" onSubmit={event => { event.preventDefault(); void commit(); }}><label>Commit message<textarea rows={2} maxLength={10000} value={commitMessage} onChange={event => setCommitMessage(event.target.value)} /></label><button className="primary" disabled={busy || loading || !git.stagedTree || !commitMessage.trim() || !git.files?.some(file => file.status !== '??' && file.status[0] !== ' ')}>Review and commit</button><small>Commits include all staged files in the repository. Unstaged edits remain on disk.</small></form>
      <details className="review-history"><summary>Branches & history</summary><form onSubmit={event => { event.preventDefault(); void gitAction({ action: 'createBranch', name: branchName }); }}><label>New branch<input value={branchName} maxLength={240} onChange={event => setBranchName(event.target.value)} /></label><button className="secondary" disabled={busy || loading || !branchName.trim()}>Create branch</button></form><button className="secondary" disabled={loading || busy} onClick={() => void gitAction({ action: 'history' })}>Load recent commits</button>{history && (history.length ? <ol>{history.map(entry => <li key={entry.id}><code>{entry.shortId}</code> {entry.subject}<small>{entry.author} · {entry.date}</small></li>)}</ol> : <p>No commits yet.</p>)}</details>
      {git.diff ? <DiffLines diff={git.diff} /> : <p className="panel-empty">{git.status ? 'No tracked text diff. Open untracked files in Editor to inspect them.' : 'No changes to commit.'}</p>}
      {git.truncated && <p className="tree-loading">The diff was truncated. Open individual files in Editor to inspect them.</p>}
    </>}
  </div>;
}
