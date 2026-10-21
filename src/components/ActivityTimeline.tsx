import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { Message, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { MarkdownView } from './MarkdownView';
import { TraceIcon, groupKindOf, toolKindOf } from './TraceIcon';

export function useSessionActivity(conversationId: string, generating: boolean) {
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const generatingRef = useRef(generating);
  generatingRef.current = generating;
  const reload = useRef<(() => void) | null>(null);
  useEffect(() => {
    let disposed = false, loading = false, queued = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cursor = 0;
    setEvents([]);
    if (!nativeAvailable || conversationId === 'new' || !api.getSessionEvents) return;
    async function load() {
      if (disposed) return;
      if (loading) { queued = true; return; }
      loading = true; clearTimeout(timer);
      let full = false, failed = false;
      try {
        const batch = await api.getSessionEvents(conversationId, cursor, 500);
        if (disposed) return;
        if (batch.length) {
          cursor = Math.max(...batch.map(event => event.seq)) + 1;
          setEvents(current => [...new Map([...current, ...batch].map(event => [event.seq, event])).values()].sort((a, b) => a.seq - b.seq));
        }
        full = batch.length === 500;
      } catch { failed = true; }
      finally {
        loading = false;
        if (!disposed && (queued || full || generatingRef.current)) {
          timer = setTimeout(() => void load(), queued || full ? 0 : failed ? 2000 : 750);
          queued = false;
        }
      }
    }
    reload.current = () => void load();
    void load();
    return () => { disposed = true; clearTimeout(timer); reload.current = null; };
  }, [conversationId]);
  useEffect(() => { reload.current?.(); }, [generating]);
  return events;
}

/** Join each event once. Run id prevents reused tool ids crossing turns. */
export function activityTools(events: SessionEvent[], messages: Message[]): Message[] {
  const results = new Map(events.filter(e => e.eventType === 'tool_result').map(e => [`${e.runId}:${e.toolCallId}`, e.payload as Record<string, any>]));
  const ended = new Set(events.filter(e => e.eventType === 'turn_end').map(e => e.runId));
  const live = new Map(messages.map(m => [m.id, m]));
  for (const message of messages) if (message.role === 'tool') {
    try { const parsed = JSON.parse(message.content); if (typeof parsed.request?.callId === 'string') live.set(`tool-${parsed.request.callId}`, message); } catch { /* Legacy tool text. */ }
  }
  const base: Message = { id: '', conversationId: '', role: 'tool', content: '', reasoning: '', createdAt: 0, status: 'complete' };
  if (!events.some(e => e.eventType === 'tool_call')) return messages.filter(m => m.role === 'tool' && !isPlanMessage(m));
  return events.filter(e => e.eventType === 'tool_call').flatMap(event => {
    const call = event.payload as Record<string, any>;
    if (isPlanStateTool(String(call.name ?? ''))) return [];
    const result = results.get(`${event.runId}:${event.toolCallId}`);
    const persisted = live.get(`tool-${event.toolCallId}`);
    if (persisted && (persisted.status !== 'streaming' || !result)) return [persisted];
    return [result ? { ...base, id: event.id, conversationId: event.conversationId, createdAt: event.createdAt, content: JSON.stringify({ request: { ...call, name: result.name ?? call.name, arguments: call.arguments, decision: result.decision ?? call.decision }, result: result.result ?? result }) } : live.get(`tool-${event.toolCallId}`) ?? { ...base, id: event.id, status: ended.has(event.runId) ? 'interrupted' as const : 'streaming' as const, content: JSON.stringify({ request: { name: call.name, arguments: call.arguments, decision: ended.has(event.runId) ? undefined : 'pending' }, result: {} }) }];
  });
}
function isPlanMessage(message: Message) {
  try { const value = JSON.parse(message.content); return isPlanStateTool(value.request?.name ?? value.name ?? ''); } catch { return false; }
}
function needsAttention(message: Message) {
  if (['streaming', 'error', 'interrupted'].includes(message.status)) return true;
  try { const value = JSON.parse(message.content); return Boolean(value.result?.error || value.result?.isError || ['error', 'failed', 'failure', 'denied', 'interrupted', 'cancelled', 'unknown', 'partial'].includes(value.result?.status) || value.request?.decision === 'denied'); } catch { return false; }
}

function toolMeta(message: Message): { name: string; args: Record<string, unknown> } {
  try {
    const value = JSON.parse(message.content);
    const request = value.request ?? value;
    return {
      name: typeof request.name === 'string' ? request.name : '',
      args: request.arguments && typeof request.arguments === 'object' ? request.arguments as Record<string, unknown> : {},
    };
  } catch { return { name: '', args: {} }; }
}

export function activityLabel(tools: Message[]): string {
  const kinds = new Set(tools.map(tool => { const meta = toolMeta(tool); return toolKindOf(meta.name, meta.args); }));
  const labels: string[] = [];
  if (kinds.has('edit')) labels.push('Edited files');
  if (kinds.has('read')) labels.push('Read files');
  if (kinds.has('search') || kinds.has('folder') || kinds.has('explore')) labels.push('Searched files');
  if (kinds.has('web')) labels.push('Browsed the web');
  if (kinds.has('command')) labels.push(tools.filter(tool => { const meta = toolMeta(tool); return toolKindOf(meta.name, meta.args) === 'command'; }).length === 1 ? 'Ran a command' : 'Ran commands');
  if (kinds.has('tool') || kinds.has('task')) labels.push('Used tools');
  return labels.map((label, index) => index ? label.toLowerCase() : label).join(', ');
}

function ActivityGroup({ tools, renderTool, live }: { tools: Message[]; renderTool: (message: Message) => ReactNode; live: boolean }) {
  const completed = tools.filter(tool => !needsAttention(tool));
  const attention = tools.filter(needsAttention);
  const kinds = completed.map(tool => { const meta = toolMeta(tool); return toolKindOf(meta.name, meta.args); });
  return <>
    {completed.length > 0 && <details className="activity-summary" open={live || undefined}>
      <summary className="card-header"><TraceIcon kind={groupKindOf(kinds)} size={16} /><span className="activity-summary-label">{activityLabel(completed)}</span><ChevronRight size={14} className="trace-chevron" aria-hidden="true" /><span className="sr-only">{completed.length} {completed.length === 1 ? 'tool used' : 'tools used'}</span></summary>
      <div className="activity-summary-rows">{completed.map(tool => <Fragment key={tool.id}>{renderTool(tool)}</Fragment>)}</div>
    </details>}
    {attention.map(tool => <Fragment key={tool.id}>{renderTool(tool)}</Fragment>)}
  </>;
}

/** Only split journal commentary when it is an exact prefix of the saved answer.
 * This keeps old, partial, or mismatched journals from duplicating model output. */
export function activitySegments(events: SessionEvent[], content: string): { seq: number; content: string }[] {
  const segments: { seq: number; content: string }[] = [];
  let remaining = content;
  for (const event of events) {
    if (event.eventType !== 'model_response') continue;
    const payload = event.payload as { content?: unknown; final?: unknown };
    if (payload.final !== false || typeof payload.content !== 'string' || !payload.content.trim()) continue;
    const text = payload.content.trim();
    if (!remaining.trimStart().startsWith(text)) return [];
    segments.push({ seq: event.seq, content: text });
    remaining = remaining.trimStart().slice(text.length);
  }
  return segments;
}

export function ActivityTimeline({ events, messages, renderMessage, renderTool, generating = false }: {
  events: SessionEvent[]; messages: Message[];
  renderMessage: (message: Message) => ReactNode; renderTool: (message: Message) => ReactNode;
  generating?: boolean;
}) {
  const assistants = messages.filter(m => m.role === 'assistant');
  const assistant = assistants[assistants.length - 1];
  const tools = useMemo(() => activityTools(events, messages), [events, messages]);
  const reasoning = assistant?.reasoning || events.filter(e => e.eventType === 'reasoning').map(e => (e.payload as { text?: string }).text ?? '').join('\n\n');
  const durations = events.filter(e => e.eventType === 'reasoning').map(e => (e.payload as { durationMs?: unknown }).durationMs);
  const measured = durations.length > 0 && durations.every(value => typeof value === 'number' && Number.isFinite(value) && value >= 0);
  const reasoningMs = measured ? (durations as number[]).reduce((sum, value) => sum + value, 0) : undefined;
  const compactions = events.filter(e => e.eventType === 'compaction');
  const [thoughtOpen, setThoughtOpen] = useState(false);
  const thoughtContent = useRef<HTMLDivElement>(null);
  const hasAnswer = Boolean(assistant?.content?.trim());
  const live = generating && Boolean(reasoning);
  const thinking = live && !hasAnswer;
  useEffect(() => { setThoughtOpen(live); }, [live]);
  useEffect(() => {
    if (live && thoughtContent.current) thoughtContent.current.scrollTop = thoughtContent.current.scrollHeight;
  }, [reasoning, live]);
  const segments = assistant && assistants.length === 1 && assistant.presentation?.format !== 'openui' ? activitySegments(events, assistant.content) : [];
  let remaining = assistant?.content ?? '';
  let lastSeq = -Infinity;
  const toolEvents = events.filter(event => event.eventType === 'tool_call' && !isPlanStateTool(String((event.payload as { name?: string }).name ?? '')));
  const groups: ReactNode[] = [];
  for (const segment of segments) {
    const before = tools.filter((_, index) => toolEvents[index] && toolEvents[index].seq > lastSeq && toolEvents[index].seq < segment.seq);
    if (before.length) groups.push(<ActivityGroup key={`tools-${segment.seq}`} tools={before} renderTool={renderTool} live={false} />);
    groups.push(<Fragment key={`commentary-${segment.seq}`}>{renderMessage({ ...assistant!, id: `${assistant!.id}-round-${segment.seq}`, content: segment.content, reasoning: '', status: 'complete', presentation: undefined, metrics: null, error: null })}</Fragment>);
    remaining = remaining.trimStart().slice(segment.content.length);
    lastSeq = segment.seq;
  }
  const trailing = segments.length ? tools.filter((_, index) => !toolEvents[index] || toolEvents[index].seq > lastSeq) : tools;
  return <div className="activity-feed">
    {reasoning && <details className="trace-thought-details" data-testid="turn-reasoning" open={thoughtOpen} onToggle={e => setThoughtOpen((e.currentTarget as HTMLDetailsElement).open)}><summary className="card-header trace-thought-pill"><TraceIcon kind="thought" size={14} /><span className={`trace-thought-label${thinking ? ' is-live' : ''}`}>{thinking ? 'Thinking…' : reasoningMs === undefined ? 'Reasoning' : `Reasoning · ${(reasoningMs / 1000).toFixed(1)}s`}</span><ChevronRight size={14} className="trace-chevron thought-chevron" aria-hidden="true" /></summary><div className="trace-thought-content" ref={thoughtContent}><MarkdownView content={reasoning} assistant streaming={thinking} /></div></details>}
    {groups}
    <ActivityGroup tools={trailing} renderTool={renderTool} live={generating} />
    {assistants.map(message => <Fragment key={message.id}>{renderMessage(message === assistant && segments.length ? { ...message, content: remaining.trimStart() } : message)}</Fragment>)}
    {compactions.map(event => <details className="activity-compaction" key={event.id}><summary>Context compacted · continuing</summary><p>{typeof (event.payload as { artifact?: unknown }).artifact === 'string' ? (event.payload as { artifact: string }).artifact : 'Older activity was summarized to make room for the next steps.'}</p></details>)}
  </div>;
}
