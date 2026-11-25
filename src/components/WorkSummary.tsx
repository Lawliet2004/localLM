import { changedFiles } from '../lib/toolActivity';
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { ChevronDown, FileDiff } from 'lucide-react';
import { DiffLines, diffCounts } from './DiffView';
import type { Message, RunRecord, SessionEvent, TaskOutcome } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { activityTools } from './ActivityTimeline';

function toolNameOf(message: Message): string {
  try { const value = JSON.parse(message.content); const name = (value.request || value).name; return typeof name === 'string' ? name : ''; }
  catch { return ''; }
}

export function WorkSummary({ messages, events = [], run, children, showActivity = true, onResume }: { messages: Message[]; events?: SessionEvent[]; run?: RunRecord | null; children: ReactNode; showActivity?: boolean; onResume?: (taskId?: string) => void }) {
  const [filesOpen, setFilesOpen] = useState(true);
  const [reviewOpen, setReviewOpen] = useState(false);
  const { tools, files } = useMemo(() => {
    const tools = activityTools(events, messages).filter(message => !isPlanStateTool(toolNameOf(message)));
    const edits = new Map<string, { path: string; kinds: Set<string>; diffs: { id: string; diff: string }[]; added: number; removed: number }>();
    for (const message of tools) for (const change of changedFiles(message)) {
      const path = change.destination ? `${change.path} → ${change.destination}` : change.path;
      const edit = edits.get(path) ?? { path, kinds: new Set<string>(), diffs: [], added: 0, removed: 0 };
      edit.kinds.add(change.kind);
      if (change.diff) {
        const counts = diffCounts(change.diff);
        edit.diffs.push({ id: `${message.id}-${edit.diffs.length}`, diff: change.diff });
        edit.added += counts.added; edit.removed += counts.removed;
      }
      edits.set(path, edit);
    }
    return { tools, files: [...edits.values()] };
  }, [messages, events]);
  const totals = files.reduce((sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }), { added: 0, removed: 0 });
  const end = [...events].reverse().find(event => event.eventType === 'turn_end');
  const start = events.find(event => event.eventType === 'turn_start' && event.runId === end?.runId);
  // Paused runs are not timed: their updatedAt - createdAt span includes idle time.
  const elapsed = run ? ['completed', 'failed', 'cancelled', 'outcome_unknown'].includes(run.status) ? run.updatedAt - run.createdAt : null : start && end ? end.createdAt - start.createdAt : null;
  const seconds = elapsed === null ? null : Math.max(0, Math.round(elapsed / 1000));
  const duration = seconds === null ? null : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  const outcome = end?.payload as TaskOutcome | undefined;
  const status = run?.status ?? outcome?.status;
  const verification = outcome?.verification?.status;
  const remaining = (outcome?.unfinishedTodos?.length ?? 0) + (outcome?.unfinishedTasks?.length ?? 0);
  const pending = outcome?.pendingWork ?? [];
  const unresolved = outcome?.unresolvedOperations ?? [];
  const resumable = Boolean(outcome?.resumeTaskId || (outcome?.partial && outcome.taskStatus == null && outcome.delivered));
  const stopReason = outcome?.stopReason === 'budget_exhausted' ? 'Work budget reached'
    : outcome?.stopReason === 'pending_work' ? 'Acceptance checks or tasks are pending'
    : outcome?.stopReason === 'restart' ? 'Recovered after an interruption'
    : outcome?.stopReason === 'failed' ? 'Response failed before the task finished' : null;
  const delivered = outcome?.delivered === true;
  const statusVerbs: Record<string, string> = { running: 'Working', paused: 'Paused', failed: 'Failed', cancelled: 'Stopped', stopped: 'Stopped', outcome_unknown: 'Outcome unknown' };
  const verb = status && !['completed', 'complete'].includes(status) ? statusVerbs[status] ?? 'Stopped' : 'Worked';
  const label = duration ? `${verb} for ${duration}` : status && !['completed', 'complete'].includes(status) ? verb : 'Work details';
  return <div className="assistant-work">
    {children}
    {showActivity && (tools.length > 0 || duration || label !== 'Work details') && <div className="work-summary" aria-label="Recorded work summary">
      <span>{label}</span>
      {tools.length > 0 && <span> · {tools.length} {tools.length === 1 ? 'action' : 'actions'}</span>}
      {files.length > 0 && <span> · {files.length} {files.length === 1 ? 'file' : 'files'} changed <span className="diff-added">+{totals.added}</span> <span className="diff-removed">−{totals.removed}</span></span>}
    </div>}
    {(resumable || unresolved.length > 0 || delivered && (outcome?.partial || verification === 'verified' || verification === 'unverified')) && <div className="task-outcome" aria-label="Task outcome">
      <div className="work-summary" role="status">
        {outcome?.partial && <span>Partial work</span>}
        {stopReason && <span> · {stopReason}</span>}
        {remaining > 0 && <span> · {remaining} unfinished {remaining === 1 ? 'task' : 'tasks'}</span>}
        {verification === 'verified' && <span>{outcome?.partial ? ' · ' : ''}Checks passed for recorded changes</span>}
        {verification === 'unverified' && <span> · Changes unverified</span>}
      </div>
      {(pending.length > 0 || unresolved.length > 0) && <details>
        <summary>Review pending work</summary>
        <ul>
          {pending.map((item, index) => <li key={index}>{item}</li>)}
          {unresolved.map(item => <li key={item.id}>{operationLabel(item.kind)} · {item.error || 'Outcome requires review'}</li>)}
        </ul>
      </details>}
      {resumable && onResume && <button type="button" className="secondary" onClick={() => onResume(outcome?.resumeTaskId ?? undefined)}>Resume task</button>}
    </div>}
    {files.length > 0 && <section className="file-change-list" aria-label="File changes">
      <div className="file-changes-header"><span className="file-changes-icon" aria-hidden="true"><FileDiff size={22} /></span><div className="file-changes-title"><span>Changed {files.length} {files.length === 1 ? 'file' : 'files'}</span><span className="file-change-counts" title="Recorded additions and deletions across these edits"><span className="diff-added">+{totals.added}</span> <span className="diff-removed">−{totals.removed}</span></span></div><button type="button" className="secondary file-changes-view" aria-expanded={reviewOpen} onClick={() => { setReviewOpen(!reviewOpen); setFilesOpen(true); }}>{reviewOpen ? 'Hide diffs' : 'View diffs'}</button></div>
      {filesOpen && <div className="file-changes-body">{files.map(edit => {
        const split = Math.max(edit.path.lastIndexOf('/'), edit.path.lastIndexOf('\\')) + 1;
        return <div className="file-change-card" key={edit.path}>
          <FileReview open={reviewOpen}>
            <summary className="file-change-heading"><span className="file-change-path" title={edit.path}><span>{edit.path.slice(0, split)}</span><strong>{edit.path.slice(split)}</strong></span><span className="file-change-counts">{edit.diffs.length ? <><span className="diff-added">+{edit.added}</span> <span className="diff-removed">−{edit.removed}</span></> : <span>{[...edit.kinds].join(', ')}</span>}</span></summary>
            <FileDiffs diffs={edit.diffs} path={edit.path} reviewOpen={reviewOpen} />
          </FileReview>
        </div>;
      })}</div>}
      <button type="button" className="file-changes-toggle" aria-expanded={filesOpen} onClick={() => setFilesOpen(!filesOpen)}>{filesOpen ? 'Collapse files' : 'Expand files'}<ChevronDown size={16} className={filesOpen ? 'is-open' : ''} aria-hidden="true" /></button>
    </section>}
  </div>;
}

const OPERATION_LABELS: Record<string, string> = { command: 'Command', edit: 'File edit', write: 'File write', tool: 'Tool call' };
function operationLabel(kind: string): string {
  return OPERATION_LABELS[kind] ?? (kind.replace(/[_-]+/g, ' ').trim() || 'Operation');
}

const FileReviewContext = createContext(false);
function FileReview({ open, children }: { open: boolean; children: ReactNode }) {
  const [expanded, setExpanded] = useState(open);
  const [inspected, setInspected] = useState(open);
  useEffect(() => { setExpanded(open); if (open) setInspected(true); }, [open]);
  return <details className="file-change-review" open={expanded} onToggle={event => { setExpanded(event.currentTarget.open); if (event.currentTarget.open) setInspected(true); }}><FileReviewContext.Provider value={expanded || inspected}>{children}</FileReviewContext.Provider></details>;
}
function FileDiffs({ diffs, path }: { diffs: { id: string; diff: string }[]; path: string; reviewOpen: boolean }) {
  const inspected = useContext(FileReviewContext);
  return inspected ? <div className="diff-viewport file-change-diff">{diffs.length ? diffs.map(diff => <DiffLines key={diff.id} diff={diff.diff} path={path} />) : <p className="muted">The tool recorded this change without a line diff.</p>}</div> : null;
}
