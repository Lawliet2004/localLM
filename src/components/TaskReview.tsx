import { useCallback, useEffect, useMemo, useState } from 'react';
import { Check, CircleAlert, FileDiff, GitBranch, History, RefreshCw, ShieldCheck, Trash2, Undo2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { TaskConflictResolution, TaskFileChange, TaskFileDiff, TaskReview as TaskReviewData, TaskWorkspace } from '../lib/types';

const STATUS_LABEL: Record<TaskFileChange['status'], string> = {
  added: 'Added',
  modified: 'Modified',
  deleted: 'Deleted',
  renamed: 'Renamed',
};

function shortSha(sha: string) { return sha.slice(0, 8); }
function humanSize(bytes: number) {
  if (bytes >= 1024 * 1024) return `${(bytes / 1048576).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${bytes} B`;
}

/** Review tab for a conversation's isolated task workspace: diff, apply,
 *  undo, checkpoints, discard. The isolated root is a copy/worktree for
 *  review and recovery — code the agent runs is not sandboxed. */
export function TaskReview({ conversation, refreshTick, onStatusChanged }: {
  conversation: string;
  refreshTick: number;
  onStatusChanged: (task: TaskWorkspace | null) => void;
}) {
  const [task, setTask] = useState<TaskWorkspace | null>(null);
  const [review, setReview] = useState<TaskReviewData | null>(null);
  const [diff, setDiff] = useState<{ path: string; value: TaskFileDiff } | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [resolutions, setResolutions] = useState<Record<string, TaskConflictResolution>>({});
  const [confirmingDiscard, setConfirmingDiscard] = useState(false);

  const loadReview = useCallback(async (active: TaskWorkspace | null) => {
    if (!active) { setReview(null); setDiff(null); return; }
    try {
      setReview(await api.taskReview(active.id));
    } catch (e) {
      setError(errorMessage(e));
      setReview(null);
    }
  }, []);

  useEffect(() => {
    if (!conversation) return;
    let disposed = false;
    setLoading(true);
    setError('');
    api.taskWorkspaceStatus(conversation)
      .then(active => {
        if (disposed) return;
        setTask(active);
        onStatusChanged(active);
        return loadReview(active);
      })
      .catch(e => { if (!disposed) setError(errorMessage(e)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversation, refreshTick]);

  async function run(action: () => Promise<void>) {
    setBusy(true); setError(''); setNotice('');
    try {
      await action();
      const active = await api.taskWorkspaceStatus(conversation);
      setTask(active);
      onStatusChanged(active);
      await loadReview(active);
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  const conflicts = useMemo(() => review?.files.filter(file => file.conflict) ?? [], [review]);
  const unresolvedConflicts = conflicts.filter(file => !resolutions[file.path]);
  const openApplies = review?.applies.filter(apply => apply.undoneAt === null) ?? [];
  const lastApply = openApplies.length ? openApplies[openApplies.length - 1] : null;

  function toggle(path: string) {
    setSelected(current => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
  }

  async function applyChanges(paths?: string[]) {
    const result = await api.taskApply(task!.id, paths, Object.keys(resolutions).length ? resolutions : undefined);
    setSelected(new Set());
    setResolutions({});
    setNotice(
      `${result.applied.length} file${result.applied.length === 1 ? '' : 's'} applied. Backup: ${result.backupDir}` +
      (result.skippedConflicts.length ? ` ${result.skippedConflicts.length} conflict${result.skippedConflicts.length === 1 ? '' : 's'} left for review.` : '') +
      (result.keptDestination.length ? ` Kept destination for ${result.keptDestination.length}.` : ''),
    );
  }

  async function openDiff(path: string) {
    setError('');
    try {
      setDiff({ path, value: await api.taskFileDiff(task!.id, path) });
    } catch (e) { setError(errorMessage(e)); }
  }

  if (loading && !task) return <p role="status">Loading…</p>;
  if (!task) {
    // After apply/discard there is no active task, but the result notice is
    // still worth showing instead of vanishing with the view.
    return notice || error ? <div className="task-review">
      {notice && <p className="task-notice" role="status">{notice}</p>}
      {error && <p className="error-banner" role="alert">{error}</p>}
    </div> : null;
  }

  const counts = {
    added: review?.files.filter(file => file.status === 'added').length ?? 0,
    modified: review?.files.filter(file => file.status === 'modified' || file.status === 'renamed').length ?? 0,
    deleted: review?.files.filter(file => file.status === 'deleted').length ?? 0,
  };

  return <div className="task-review">
    <div className="task-review-head">
      <span className="task-review-badge"><ShieldCheck size={13} /> Isolated task</span>
      <span className="task-review-kind">{task.kind === 'git-worktree' ? 'Git worktree' : 'Snapshot copy'}</span>
      {task.branch && <span className="task-review-branch" title={`Task branch ${task.branch}`}><GitBranch size={12} /> {task.branch}</span>}
    </div>
    <p className="task-review-paths" title={`Source: ${task.sourceRoot}\nIsolated: ${task.isolatedRoot}`}>
      {task.kind === 'snapshot'
        ? 'Working on a copy for review and recovery — this is not a security sandbox.'
        : <>Agent changes land in <code>{task.isolatedRoot}</code>, kept apart from the source folder.</>}
    </p>

    {review && <>
      <p className="task-review-summary">
        <strong>{review.files.length}</strong> change{review.files.length === 1 ? '' : 's'}
        {` · ${counts.added} added · ${counts.modified} modified · ${counts.deleted} deleted`}
        {conflicts.length > 0 && <span className="task-conflict-count"><CircleAlert size={12} /> {conflicts.length} conflict{conflicts.length === 1 ? '' : 's'}</span>}
      </p>

      {review.exclusions.length > 0 && <details className="task-exclusions">
        <summary>{review.exclusions.length} item{review.exclusions.length === 1 ? '' : 's'} not carried into the task</summary>
        <ul>
          {review.exclusions.map(item => <li key={item.path}><code>{item.path}</code> — {item.reason}</li>)}
        </ul>
      </details>}

      {review.files.length === 0 && <p className="panel-empty">No changes yet in the isolated workspace.</p>}
      {review.files.length > 0 && <ul className="task-file-list">
        {review.files.map(file => <li key={file.path} className={file.conflict ? 'task-file conflict' : 'task-file'}>
          <label className="task-file-check">
            <input
              type="checkbox"
              aria-label={`Select ${file.path}`}
              checked={selected.has(file.path)}
              disabled={busy}
              onChange={() => toggle(file.path)}
            />
          </label>
          <button type="button" className="task-file-name" onClick={() => void openDiff(file.path)} title={file.oldPath ? `${file.oldPath} → ${file.path}` : file.path}>
            <FileDiff size={13} aria-hidden="true" />
            <span>{file.path}</span>
            <small>{humanSize(file.size)}{file.binary ? ' · binary' : ''}</small>
          </button>
          <span className={`task-status task-status-${file.status}`}>{STATUS_LABEL[file.status]}</span>
          {file.conflict && <span className="task-conflict-chip" title="The source file changed since the task started">
            <CircleAlert size={12} /> conflict
          </span>}
          {file.conflict && <select
            aria-label={`Resolve conflict for ${file.path}`}
            value={resolutions[file.path] ?? ''}
            disabled={busy}
            onChange={event => setResolutions(current => ({ ...current, [file.path]: event.target.value as TaskConflictResolution }))}
          >
            <option value="">Resolve…</option>
            <option value="take-task">Use task version</option>
            <option value="keep-destination">Keep current file</option>
          </select>}
        </li>)}
      </ul>}

      {diff && <div className="task-diff-view">
        <p className="task-diff-title"><code>{diff.path}</code>
          {diff.value.truncated && <span> — truncated at 256 KiB</span>}
          <button type="button" className="icon-button" aria-label="Close diff" onClick={() => setDiff(null)}>×</button>
        </p>
        {diff.value.binary && <p className="panel-empty">Binary file — {diff.value.reason ?? 'no diff to show'}.</p>}
        {diff.value.reason && !diff.value.binary && <p className="panel-empty">{diff.value.reason}</p>}
        {diff.value.text && <pre>{diff.value.text}</pre>}
      </div>}

      <div className="task-actions">
        <button type="button" className="secondary" disabled={busy || !review.files.length || unresolvedConflicts.length > 0}
          title={unresolvedConflicts.length ? 'Resolve every conflict first' : 'Write the task changes back to the workspace'}
          onClick={() => void run(() => applyChanges())}>
          <Check size={14} /> Apply all
        </button>
        <button type="button" className="secondary" disabled={busy || selected.size === 0 || unresolvedConflicts.some(file => selected.has(file.path))}
          title="Apply only the checked files"
          onClick={() => void run(() => applyChanges([...selected]))}>
          Apply selected ({selected.size})
        </button>
        {lastApply && <button type="button" className="secondary" disabled={busy}
          title="Undo restores files only. It cannot reverse messages sent, packages installed, or remote changes."
          onClick={() => void run(async () => {
            const undone = await api.taskUndoApply(task.id, lastApply.id);
            setNotice(`Restored ${undone.restored.length} file${undone.restored.length === 1 ? '' : 's'}.${undone.conflicts.length ? ` ${undone.conflicts.length} file${undone.conflicts.length === 1 ? ' was' : 's were'} edited since and left alone.` : ''}`);
          })}>
          <Undo2 size={14} /> Undo last apply
        </button>}
        <button type="button" className="secondary" disabled={busy} onClick={() => void run(async () => { await loadReview(task); setNotice('Refreshed.'); })}>
          <RefreshCw size={14} /> Refresh
        </button>
        {!confirmingDiscard && <button type="button" className="secondary danger" disabled={busy} onClick={() => setConfirmingDiscard(true)}>
          <Trash2 size={14} /> Discard task
        </button>}
      </div>
      {confirmingDiscard && <div className="task-discard-confirm" role="group" aria-label="Confirm discard">
        <p>Discard the isolated workspace and its checkpoints? Nothing has been written to your files{task.kind === 'git-worktree' ? ` — the task branch ${task.branch} is deleted` : ''}.</p>
        <button type="button" className="secondary" disabled={busy} onClick={() => setConfirmingDiscard(false)}>Keep it</button>
        <button type="button" className="primary danger" disabled={busy}
          onClick={() => void run(async () => { await api.taskDiscard(task.id); setConfirmingDiscard(false); setDiff(null); setNotice('Task workspace discarded.'); })}>
          Discard workspace
        </button>
      </div>}

      {review.checkpoints.length > 0 && <div className="task-checkpoints">
        <p className="task-section-label"><History size={13} /> Checkpoints</p>
        <ul>
          {review.checkpoints.map(cp => <li key={cp.commit ?? 'uncommitted'}>
            <span className="task-checkpoint-label">{cp.label}</span>
            {cp.commit && <code title={cp.commit}>{shortSha(cp.commit)}</code>}
            {cp.uncommitted ? <span className="task-checkpoint-note">not saved</span> :
              cp.commit ? <button type="button" className="secondary task-restore" disabled={busy}
                onClick={() => void run(async () => { await api.taskRestoreCheckpoint(task.id, cp.commit!); setNotice(`Restored checkpoint ${cp.label}.`); })}>Restore</button> : null}
          </li>)}
        </ul>
      </div>}
    </>}

    {notice && <p className="task-notice" role="status">{notice}</p>}
    {error && <p className="error-banner" role="alert">{error}</p>}
  </div>;
}
