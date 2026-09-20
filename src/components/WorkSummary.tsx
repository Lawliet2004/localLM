import { FileIcon } from './CommandRunCard';
import type { ReactNode } from 'react';
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

export function WorkSummary({ messages, run, children }: { messages: Message[]; run?: RunRecord | null; children: ReactNode }) {
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
  const seconds = run && ['completed', 'failed', 'cancelled', 'outcome_unknown'].includes(run.status) ? Math.max(0, Math.round((run.updatedAt - run.createdAt) / 1000)) : null;
  const duration = seconds === null ? null : seconds >= 60 ? `${Math.floor(seconds / 60)}m ${seconds % 60}s` : `${seconds}s`;
  return <div className="assistant-work">
    {(tools.length > 0 || duration) && <details className="work-summary"><summary>{duration ? `${run?.status === 'completed' ? 'Worked' : 'Stopped'} for ${duration}` : 'Work details'}</summary><p>{tools.length} recorded tool {tools.length === 1 ? 'action' : 'actions'}{run ? ` · ${run.status.replace(/_/g, ' ')}` : ''}. Expand the tool entries above to inspect their arguments and results.</p></details>}
    {children}
    {edits.length > 0 && <div className="file-change-list" aria-label="File changes">{edits.map(edit => <div className="file-change-card" key={edit.id}>
      <div className="file-change-heading"><FileIcon filename={edit.path} size={16} /><div><strong>Edited {edit.path.split(/[\\/]/).pop()}</strong><p><span className="diff-added">+{edit.added}</span> <span className="diff-removed">−{edit.removed}</span></p></div></div>
      <details className="file-change-review"><summary>Review</summary><p>{edit.path}</p><pre className="tool-diff">{edit.diff}</pre></details>
    </div>)}</div>}
  </div>;
}
