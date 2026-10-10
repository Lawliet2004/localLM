import { FileIcon } from './CommandRunCard';
import { ChevronRight } from 'lucide-react';
import { useEffect, useRef, type ReactNode } from 'react';
import type { Message, RunRecord } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';

function toolNameOf(message: Message): string {
  try {
    const value = JSON.parse(message.content);
    const request = value.request || value;
    return typeof request.name === 'string' ? request.name : '';
  } catch {
    return '';
  }
}

export function WorkSummary({ messages, run, durationMs, activity, running = false, initiallyOpen = false, steps = 0, children }: {
  messages: Message[]; run?: RunRecord | null; durationMs?: number | null;
  activity?: ReactNode; running?: boolean; initiallyOpen?: boolean; steps?: number; children: ReactNode;
}) {
  const details = useRef<HTMLDetailsElement>(null);
  useEffect(() => { if (details.current) details.current.open = running || initiallyOpen; }, [running, initiallyOpen]);
  const tools = messages.filter(message => message.role === 'tool' && !isPlanStateTool(toolNameOf(message)));
  const edits = tools.flatMap(message => {
    if (message.status !== 'complete') return [];
    try {
      const { request, result } = JSON.parse(message.content);
      if (request?.connector !== 'Workspace' || request?.name !== 'edit_file' || result?.isError || request?.decision === 'denied' || typeof result?.diff !== 'string' || typeof request?.arguments?.path !== 'string') return [];
      let added = 0, removed = 0, inHunk = false;
      for (const line of result.diff.split('\n')) {
        if (line.startsWith('@@')) inHunk = true;
        else if (inHunk && line.startsWith('+')) added++;
        else if (inHunk && line.startsWith('-')) removed++;
      }
      return [{ id: message.id, path: request.arguments.path as string, diff: result.diff as string, added, removed }];
    } catch { return []; }
  });
  const elapsed = run && ['completed', 'failed', 'cancelled', 'outcome_unknown'].includes(run.status)
    ? run.updatedAt - run.createdAt : durationMs;
  const seconds = elapsed == null ? null : Math.max(0, Math.round(elapsed / 1000));
  const duration = seconds === null ? null : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  const stopped = run && run.status !== 'completed';
  return <div className="assistant-work">
    {(activity || tools.length > 0 || duration) && <details ref={details} className={`work-summary${running ? ' is-running' : ''}`}>
      <summary>
        {running && <span className="work-summary-pulse" aria-hidden="true" />}
        <span className="work-summary-label">{running
          ? duration ? <>Working for <span className="work-duration">{duration}</span></> : 'Working…'
          : duration ? <>{stopped ? 'Stopped' : 'Worked'} for <span className="work-duration">{duration}</span></>
          : tools.length ? 'Work details' : 'Worked'}</span>
        {steps > 0 && <span className="work-summary-meta">{steps} {steps === 1 ? 'step' : 'steps'}</span>}
        <ChevronRight size={14} aria-hidden="true" />
      </summary>
      <div className="work-summary-content">{activity || <p>{tools.length} recorded tool {tools.length === 1 ? 'action' : 'actions'}{run ? ` · ${run.status.replace(/_/g, ' ')}` : ''}.</p>}</div>
    </details>}
    {children}
    {edits.length > 0 && <div className="file-change-list" aria-label="File changes">
      <div className="file-change-list-heading"><strong>Edited {edits.length} {edits.length === 1 ? 'file' : 'files'}</strong><span><span className="diff-added">+{edits.reduce((total, edit) => total + edit.added, 0)}</span> <span className="diff-removed">−{edits.reduce((total, edit) => total + edit.removed, 0)}</span></span></div>
      {edits.map(edit => <div className="file-change-card" key={edit.id}>
      <div className="file-change-heading"><FileIcon filename={edit.path} size={16} /><div><strong>{edit.path}</strong><p><span className="diff-added">+{edit.added}</span> <span className="diff-removed">−{edit.removed}</span></p></div></div>
      <details className="file-change-review"><summary>Review</summary><p>{edit.path}</p><pre className="tool-diff">{edit.diff}</pre></details>
    </div>)}</div>}
  </div>;
}
