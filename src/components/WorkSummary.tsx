import { useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { DiffLines, diffCounts } from './DiffView';
import type { Message, RunRecord, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { activityTools } from './ActivityTimeline';
import { toolFileChanges, useFullMutationResults } from '../lib/toolResult';

function toolNameOf(message: Message): string {
  try { const value = JSON.parse(message.content); const name = (value.request || value).name; return typeof name === 'string' ? name : ''; }
  catch { return ''; }
}

export function WorkSummary({ messages, events = [], run, children, showActivity = true }: { messages: Message[]; events?: SessionEvent[]; run?: RunRecord | null; children: ReactNode; showActivity?: boolean }) {
  const [filesOpen, setFilesOpen] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  const { messages: tools, error: artifactError } = useFullMutationResults(activityTools(events, messages).filter(message => !isPlanStateTool(toolNameOf(message))));
  const edits = new Map<string, { path: string; diffs: { id: string; diff: string }[]; incomplete: boolean; added: number; removed: number }>();
  for (const message of tools) {
    for (const change of toolFileChanges(message)) {
      const edit = edits.get(change.path) ?? { path: change.path, diffs: [], incomplete: false, added: 0, removed: 0 };
      if (change.diff !== undefined) {
        const { added, removed } = diffCounts(change.diff);
        edit.diffs.push({ id: `${message.id}-${change.path}`, diff: change.diff });
        edit.added += added; edit.removed += removed;
      } else edit.incomplete = true;
      edits.set(change.path, edit);
    }
  }
  const files = [...edits.values()];
  const totals = files.reduce((sum, file) => ({ added: sum.added + file.added, removed: sum.removed + file.removed }), { added: 0, removed: 0 });
  const totalsKnown = files.every(file => !file.incomplete);
  const end = [...events].reverse().find(event => event.eventType === 'turn_end');
  const start = events.find(event => event.eventType === 'turn_start' && event.runId === end?.runId);
  const elapsed = run ? ['completed', 'failed', 'cancelled', 'outcome_unknown', 'paused'].includes(run.status) ? run.updatedAt - run.createdAt : null : start && end ? end.createdAt - start.createdAt : null;
  const seconds = elapsed === null ? null : Math.max(0, Math.round(elapsed / 1000));
  const duration = seconds === null ? null : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  const status = run?.status ?? (end?.payload as { status?: string } | undefined)?.status;
  const verificationEvent = events.filter(event => event.eventType === 'verification_status').sort((a, b) => b.seq - a.seq)[0];
  const verification = verificationEvent?.payload as { status?: string; message?: string } | undefined;
  const verb = status && !['completed', 'complete'].includes(status) ? status === 'paused' ? 'Paused' : 'Stopped' : 'Worked';
  return <div className="assistant-work">
    {showActivity && (tools.length > 0 || duration) && <details className="work-summary"><summary>{duration ? `${verb} for ${duration}` : 'Work details'}</summary><p>{tools.length} recorded tool {tools.length === 1 ? 'action' : 'actions'}{run ? ` · ${run.status.replace(/_/g, ' ')}` : ''}. Expand the activity rows to inspect their arguments and results.</p></details>}
    {children}
    {typeof verification?.message === 'string' && <p className="verification-outcome" role="status">{verification.message}</p>}
    {files.length > 0 && <section className="file-change-list" aria-label="File changes">
      <div className="file-changes-header"><button type="button" className="file-changes-disclosure" aria-expanded={filesOpen} onClick={() => setFilesOpen(!filesOpen)}><ChevronRight size={16} className={filesOpen ? 'is-open' : ''} aria-hidden="true" /><span>{files.length} {files.length === 1 ? 'file' : 'files'} changed</span>{totalsKnown && <span className="file-change-counts" title="Recorded additions and deletions across these edits"><span className="diff-added">+{totals.added}</span> <span className="diff-removed">−{totals.removed}</span></span>}</button>{filesOpen && <button type="button" className="secondary file-changes-view" aria-expanded={reviewOpen} onClick={() => setReviewOpen(!reviewOpen)}>{reviewOpen ? 'Hide changes' : 'View changes'}</button>}</div>
      {filesOpen && <div className="file-changes-body">{files.map(edit => {
        const split = Math.max(edit.path.lastIndexOf('/'), edit.path.lastIndexOf('\\')) + 1;
        return <div className="file-change-card" key={edit.path}>
          <details className="file-change-review" open={reviewOpen || undefined}>
            <summary className="file-change-heading"><span className="file-change-path" title={edit.path}><span>{edit.path.slice(0, split)}</span><strong>{edit.path.slice(split)}</strong></span>{!edit.incomplete && <span className="file-change-counts"><span className="diff-added">+{edit.added}</span> <span className="diff-removed">−{edit.removed}</span></span>}</summary>
            <div className="diff-viewport file-change-diff">{edit.diffs.map(diff => <DiffLines key={diff.id} diff={diff.diff} path={edit.path} />)}{edit.incomplete && <p className="muted">Changed file; full diff and line totals are unavailable here.</p>}</div>
          </details>
        </div>;
      })}</div>}

    </section>}
    {artifactError && <p role="alert">Could not load full file changes: {artifactError}</p>}
  </div>;
}
