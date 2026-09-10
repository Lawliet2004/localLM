import { useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { RunEvent, RunRecord, SubagentRun } from '../lib/types';

// ponytail: read-only view over existing runs/run_events. No replay execution;
// export covers portability, fork lives in SessionsPanel.
function preview(value: unknown): string {
  const text = JSON.stringify(value);
  return text.length > 2048 ? `${text.slice(0, 2048)}… (truncated, full event in database)` : text;
}

export function Trajectory({ conversationId }: { conversationId: string | null }) {
  const [open, setOpen] = useState(false);
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [runId, setRunId] = useState<string | null>(null);
  const [events, setEvents] = useState<RunEvent[]>([]);
  const [agents, setAgents] = useState<SubagentRun[]>([]);
  const [filter, setFilter] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [loading, setLoading] = useState(false);

  async function load() {
    if (!nativeAvailable || !conversationId || loading) return;
    setLoading(true); setError(''); setNotice('');
    try {
      const [allRuns, children] = await Promise.all([
        api.getConversationRuns(conversationId),
        api.listSubagentRuns(conversationId).catch(() => [] as SubagentRun[]),
      ]);
      setRuns(allRuns);
      setAgents(children);
      const active = allRuns.find(run => run.checkpoint !== 'subagent' && run.status !== 'completed' && run.status !== 'cancelled' && run.status !== 'failed')
        ?? [...allRuns].reverse().find(run => run.checkpoint !== 'subagent')
        ?? allRuns[allRuns.length - 1]
        ?? null;
      const target = runId && allRuns.some(run => run.id === runId) ? runId : active?.id ?? null;
      setRunId(target);
      setEvents(target ? await api.getRunEvents(target) : []);
    } catch (e) { setError(errorMessage(e)); } finally { setLoading(false); }
  }

  async function select(id: string) {
    setRunId(id); setError('');
    try { setEvents(await api.getRunEvents(id)); }
    catch (e) { setError(errorMessage(e)); }
  }

  async function interrupt(childRunId: string) {
    setError('');
    try {
      await api.interruptSubagent(childRunId);
      setNotice(`Interrupt sent to ${childRunId}.`);
      await load();
    } catch (e) { setError(errorMessage(e)); }
  }

  async function compact() {
    if (!conversationId) return;
    setError(''); setNotice('');
    try {
      const checkpoint = await api.compactConversation(conversationId);
      setNotice(`Compacted into artifact ${checkpoint.artifactId}. Audits stay in the database.`);
    } catch (e) { setError(errorMessage(e)); }
  }

  const query = filter.toLowerCase();
  const visible = query
    ? events.filter(event => event.eventType.toLowerCase().includes(query) || JSON.stringify(event.payload).toLowerCase().includes(query))
    : events;
  const active = runs.find(run => run.id === runId);

  return (
    <details className="trajectory" onToggle={event => { if ((event.target as HTMLDetailsElement).open && !open) { setOpen(true); void load(); } else if (!(event.target as HTMLDetailsElement).open) setOpen(false); }}>
      <summary>Trajectory{active ? ` · ${active.status} · ${events.length} events` : ''}{agents.length ? ` · ${agents.length} subagents` : ''}</summary>
      <div className="trajectory-body">
        <div className="row-actions">
          <label>Run{' '}
            <select aria-label="Select run" value={runId ?? ''} onChange={e => void select(e.target.value)}>
              <option value="">—</option>
              {runs.map(run => <option key={run.id} value={run.id}>{run.id.slice(0, 13)} · {run.status}{run.checkpoint === 'subagent' ? ' · subagent' : ''}</option>)}
            </select></label>
          <input aria-label="Filter trajectory events" type="search" placeholder="Filter by type or payload…" value={filter} onChange={e => setFilter(e.target.value)} />
          <button type="button" className="secondary" disabled={loading || !conversationId} onClick={() => void load()}>{loading ? 'Loading…' : 'Refresh'}</button>
          <button type="button" className="secondary" disabled={!conversationId} onClick={() => void compact()}>Compact history</button>
        </div>
        {error && <p role="alert">{error}</p>}
        {notice && <p role="status">{notice}</p>}
        {!conversationId && <p>Start a conversation to see its run trajectory.</p>}
        {agents.length > 0 && (
          <ul className="agent-tree">{agents.map(agent => (
            <li key={agent.id}>
              <span>{'·'.repeat(Math.min(agent.depth, 6))} {agent.label}</span>{' '}
              <small>{agent.status} · depth {agent.depth}</small>{' '}
              {agent.status === 'running' && <button type="button" className="secondary" onClick={() => void interrupt(agent.childRunId)}>Interrupt</button>}
              {agent.error && <small role="alert">{agent.error}</small>}
            </li>
          ))}</ul>
        )}
        {conversationId && !loading && !events.length && !error && <p>No run recorded for this conversation yet.</p>}
        {visible.map(event => (
          <details key={`${event.runId}:${event.seq}`} className="trajectory-event">
            <summary>#{event.seq} · {event.eventType}{event.toolCallId ? ` · ${event.toolCallId}` : ''}</summary>
            <pre>{preview(event.payload)}</pre>
          </details>
        ))}
        {open && query && !visible.length && events.length > 0 && <p>No events match this filter.</p>}
      </div>
    </details>
  );
}
