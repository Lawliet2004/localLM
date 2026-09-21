import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Checkpoint, CheckpointDiff, RevertResult } from '../lib/types';

/** Per-turn workspace checkpoints (checkpoints.rs): review what a turn changed and revert it. */
export function TurnChanges({ conversationId, revision, busy }: { conversationId: string | null; revision: string; busy: boolean }) {
  const [checkpoints, setCheckpoints] = useState<Checkpoint[]>([]);
  const [open, setOpen] = useState<{ id: string; diff: CheckpointDiff } | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [result, setResult] = useState<RevertResult | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState('');
  const [reload, setReload] = useState(0);
  const [enabled, setEnabled] = useState<boolean | null>(null);
  useEffect(() => {
    if (!nativeAvailable) return;
    api.checkpointSettings().then(settings => setEnabled(settings.enabled)).catch(() => setEnabled(null));
  }, []);
  useEffect(() => {
    if (!nativeAvailable || !conversationId) { setCheckpoints([]); return; }
    let disposed = false;
    api.listCheckpoints(conversationId)
      .then(list => { if (!disposed) setCheckpoints(list.filter(item => item.afterCommit)); })
      .catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [conversationId, revision, reload]);
  async function act(action: () => Promise<void>) {
    setWorking(true); setError('');
    try { await action(); } catch (e) { setError(errorMessage(e)); } finally { setWorking(false); }
  }
  if (!conversationId) return null;
  return <section aria-label="Turn changes" className="turn-changes">
    <h3>Changes by turn</h3>
    {error && <p role="alert" className="error-banner">{error}</p>}
    {result && <p role="status">
      Reverted {result.restored.length + result.deleted.length} file(s).
      {result.conflicts.length > 0 && <> Left unchanged because they were edited after that turn: {result.conflicts.join(', ')}.</>}
    </p>}
    {checkpoints.length === 0 && <p className="panel-empty">No agent file changes recorded for this conversation.</p>}
    <ul className="turn-change-list">{checkpoints.map(item => <li key={item.id}>
      <div className="turn-change-row">
        <span>{new Date(item.createdAt).toLocaleString()} · {item.label === 'turn' ? 'Agent turn' : item.label} · {item.filesChanged ?? 0} file(s){item.status !== 'complete' && ` · ${item.status.replace('_', ' ')}`}</span>
        <button type="button" className="secondary" disabled={working} onClick={() => void act(async () => { setOpen(open?.id === item.id ? null : { id: item.id, diff: await api.checkpointDiff(item.id) }); })}>{open?.id === item.id ? 'Hide' : 'Review'}</button>
        {confirming === item.id
          ? <button type="button" className="danger" disabled={busy || working} onClick={() => void act(async () => { setConfirming(null); setResult(await api.revertCheckpoint(item.id)); setOpen(null); setReload(value => value + 1); })}>Confirm revert</button>
          : <button type="button" className="secondary" disabled={busy || working || item.status === 'reverted'} onClick={() => { setResult(null); setConfirming(item.id); }}>Revert</button>}
      </div>
      {Array.isArray(item.excluded) && item.excluded.length > 0 && <small>{item.excluded.length} path(s) were not captured (large files or nested repositories) and are never changed by a revert.</small>}
      {open?.id === item.id && <>
        <ul aria-label="Changed files">{open.diff.changes.map(change => <li key={change.path}><code>{change.status}</code> {change.path}</li>)}</ul>
        <pre className="turn-change-diff">{open.diff.diff}{open.diff.truncated ? '\n…diff truncated…' : ''}</pre>
      </>}
    </li>)}</ul>
    <div className="form-footer">
      <label className="checkbox-row"><input type="checkbox" aria-label="Record checkpoints" checked={enabled ?? true} disabled={enabled === null || working || busy}
        onChange={e => { const next = e.target.checked; void act(async () => { setEnabled((await api.saveCheckpointSettings({ enabled: next })).enabled); setReload(value => value + 1); }); }} />Record checkpoints (copies of changed workspace files stay on this computer)</label>
      <button type="button" className="secondary" disabled={working || busy} onClick={() => void act(async () => { await api.clearCheckpoints(); setResult(null); setOpen(null); setReload(value => value + 1); })}>Delete all checkpoints</button>
    </div>
  </section>;
}
