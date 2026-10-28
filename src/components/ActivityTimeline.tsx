import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { Message, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { MarkdownView } from './MarkdownView';
import { TraceIcon, groupKindOf, toolKindOf } from './TraceIcon';
import { toolRecord, toolOutcome, toolRowKey } from '../lib/toolResult';
import { toolStreamId, withLiveRounds } from '../lib/agentTranscript';

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
    let unlisten: (() => void) | undefined;
    // Subscribe before recovery loading. Both paths read the same committed
    // journal; polling also covers missed notifications and older backends.
    if (api.onSessionActivity) void api.onSessionActivity(conversationId, seq => {
      if (!disposed && seq >= cursor) void load();
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => { /* Polling remains available. */ });
    void load();
    return () => { disposed = true; unlisten?.(); clearTimeout(timer); reload.current = null; };
  }, [conversationId]);
  useEffect(() => { reload.current?.(); }, [generating]);
  return events.filter(event => event.conversationId === conversationId);
}

/** Duplicate delivery must not duplicate an operation or move it in the log. */
function toolCalls(events: SessionEvent[]) {
  const seen = new Set<string>();
  return [...events].sort((a, b) => a.seq - b.seq).filter(event => {
    if (event.eventType !== 'tool_call' || isPlanStateTool(String((event.payload as { name?: string }).name ?? ''))) return false;
    const key = JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId ?? event.id]);
    if (seen.has(key)) return false;
    seen.add(key); return true;
  });
}

const replay = new WeakMap<SessionEvent, { result?: SessionEvent; ended: boolean; message: Message }>();

/** Run identity and call intervals prevent reused provider IDs crossing runs.
 * Immutable event objects also let completed replay rows retain their identity. */
export function activityTools(events: SessionEvent[], messages: Message[]): Message[] {
  return activityToolEntries(events, messages).map(entry => entry.message);
}

/** Every tool carries its own journal position. Unjournaled legacy rows use
 * recorded timestamps; array positions are never interpreted as call IDs. */
function activityToolEntries(events: SessionEvent[], messages: Message[]): { message: Message; seq: number }[] {
  const ordered = [...events].sort((a, b) => a.seq - b.seq);
  const fallbackSeq = (message: Message) => {
    const next = ordered.find(event => event.createdAt > message.createdAt);
    return next ? next.seq - 0.5 : (ordered[ordered.length - 1]?.seq ?? 0) + 0.5;
  };
  const calls = toolCalls(events);
  if (!calls.length) return messages.filter(message => message.role === 'tool' && !isPlanMessage(message)).map(message => ({ message, seq: fallbackSeq(message) }));
  const callKey = (event: SessionEvent) => JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId]);
  const results = new Map(ordered.filter(event => event.eventType === 'tool_result').map(event => [callKey(event), event]));
  const ended = new Set(events.filter(event => event.eventType === 'turn_end').map(event => event.runId));
  const candidates = new Map<string, Message[]>();
  for (const message of messages) if (message.role === 'tool') {
    const request = toolRecord(message).request;
    for (const key of new Set([message.id, typeof request?.callId === 'string' ? `tool-${request.callId}` : message.id])) {
      const list = candidates.get(key) ?? []; list.push(message); candidates.set(key, list);
    }
  }
  const callsById = new Map<string | null | undefined, SessionEvent[]>();
  for (const call of calls) { const list = callsById.get(call.toolCallId) ?? []; list.push(call); callsById.set(call.toolCallId, list); }
  const matched = new Set<Message>();
  const reconstructed = calls.map(event => {
    const siblings = callsById.get(event.toolCallId)!;
    const next = siblings[siblings.indexOf(event) + 1];
    const persisted = candidates.get(`tool-${event.toolCallId}`)?.find(message => {
      const request = toolRecord(message).request;
      if (request?.runId && request.runId !== event.runId) return false;
      if (request?.stepId && event.stepId) return request.stepId === event.stepId;
      if (request?.runId && siblings.filter(call => call.runId === event.runId).length === 1) return true;
      return siblings.length === 1 || message.createdAt >= event.createdAt && (!next || message.createdAt < next.createdAt);
    });
    if (persisted) matched.add(persisted);
    const resultEvent = results.get(callKey(event));
    if (persisted && (persisted.status !== 'streaming' || !resultEvent)) return { message: persisted, seq: event.seq };
    const finished = ended.has(event.runId);
    const cached = replay.get(event);
    if (cached && cached.result === resultEvent && cached.ended === finished) return { message: cached.message, seq: event.seq };
    const call = event.payload as Record<string, any>;
    const result = resultEvent?.payload as Record<string, any> | undefined;
    const message: Message = {
      id: persisted?.id ?? (event.runId && event.stepId && event.toolCallId ? toolStreamId({ conversationId: event.conversationId, runId: event.runId, stepId: event.stepId, toolCallId: event.toolCallId }) : event.id), conversationId: event.conversationId, role: 'tool', reasoning: '', createdAt: event.createdAt,
      status: result ? 'complete' : finished ? 'interrupted' : 'streaming',
      content: JSON.stringify({ request: { ...call, callId: event.toolCallId, runId: event.runId, stepId: event.stepId, name: result?.name ?? call.name, arguments: call.arguments, decision: result?.decision ?? (finished ? undefined : call.decision) }, result: result ? result.result ?? result : {} }),
    };
    replay.set(event, { result: resultEvent, ended: finished, message });
    return { message, seq: event.seq };
  });
  // Journal notifications can lag the message stream, and older history can
  // have only partial journals. Never hide calls just because one event exists.
  return [...reconstructed, ...messages.filter(message => message.role === 'tool' && !isPlanMessage(message) && !matched.has(message)).map(message => ({ message, seq: fallbackSeq(message) }))].sort((a, b) => a.seq - b.seq);
}
function isPlanMessage(message: Message) {
  const value = toolRecord(message); return isPlanStateTool(value.request?.name ?? value.name ?? '');
}
function needsAttention(message: Message) {
  return toolOutcome(message).needsAttention;
}
function toolMeta(message: Message): { name: string; args: Record<string, unknown> } {
  const value = toolRecord(message), request = value.request ?? value;
  return { name: typeof request.name === 'string' ? request.name : '', args: request.arguments && typeof request.arguments === 'object' ? request.arguments : {} };
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

/** Only adjacent successful exploration can fold. Consequential work and
 * attention states form hard boundaries, so chronology survives collapsing. */
function ActivityGroup({ tools, renderTool }: { tools: Message[]; renderTool: (message: Message) => ReactNode; live: boolean }) {
  const rows: ReactNode[] = [];
  let exploration: Message[] = [];
  const flush = () => {
    if (!exploration.length) return;
    const group = exploration;
    exploration = [];
    if (group.length === 1) { rows.push(<Fragment key={toolRowKey(group[0])}>{renderTool(group[0])}</Fragment>); return; }
    const kinds = group.map(tool => { const meta = toolMeta(tool); return toolKindOf(meta.name, meta.args); });
    const paths = group.map(tool => toolMeta(tool).args.path);
    const fileCount = new Set(paths).size;
    const counts = [
      [kinds.filter(kind => kind === 'search').length, 'search', 'searches'],
      [new Set(group.filter((_, index) => kinds[index] === 'read').map(tool => toolMeta(tool).args.path ?? tool.id)).size, 'file', 'files'],
      [kinds.filter(kind => kind === 'folder' || kind === 'explore').length, 'folder', 'folders'],
    ] as const;
    const summary = counts.filter(([count]) => count > 0).map(([count, one, many]) => `${count} ${count === 1 ? one : many}`).join(', ');
    const label = paths.every(path => typeof path === 'string') && kinds.every(kind => kind === 'read')
      ? `Read ${fileCount} ${fileCount === 1 ? 'file' : 'files'}`
      : `Explore · ${summary}`;
    rows.push(<details className="activity-summary" key={toolRowKey(group[0])}>
      <summary className="card-header"><TraceIcon kind={groupKindOf(kinds)} /><span className="activity-summary-label">{label}</span><ChevronRight size={14} className="trace-chevron" aria-hidden="true" /><span className="sr-only">{group.length} tools used</span></summary>
      <div className="activity-summary-rows">{group.map(tool => <Fragment key={toolRowKey(tool)}>{renderTool(tool)}</Fragment>)}</div>
    </details>);
  };
  for (const tool of tools) {
    const meta = toolMeta(tool);
    if (!needsAttention(tool) && ['read', 'search', 'folder', 'explore'].includes(toolKindOf(meta.name, meta.args))) exploration.push(tool);
    else { flush(); rows.push(<Fragment key={toolRowKey(tool)}>{renderTool(tool)}</Fragment>); }
  }
  flush();
  return <>{rows}</>;
}

/** Only split journal commentary when it is an exact prefix of the saved answer.
 * This keeps old, partial, or mismatched journals from duplicating model output. */
export function activitySegments(events: SessionEvent[], content: string, separated = false): { seq: number; content: string }[] {
  const segments: { seq: number; content: string }[] = [];
  let remaining = content;
  for (const event of events) {
    if (event.eventType !== 'model_response') continue;
    const payload = event.payload as { content?: unknown; final?: unknown };
    if (payload.final !== false || typeof payload.content !== 'string' || !payload.content.trim()) continue;
    const text = payload.content.trim();
    if (!separated && !remaining.trimStart().startsWith(text)) return [];
    segments.push({ seq: event.seq, content: text });
    if (!separated) remaining = remaining.trimStart().slice(text.length);
  }
  return segments;
}

/** A round keeps its disclosure identity when live text becomes journaled. */
function Thought({ text, durationMs, live = false, thinking = false }: { text: string; durationMs?: number; live?: boolean; thinking?: boolean }) {
  const [open, setOpen] = useState(live);
  const content = useRef<HTMLDivElement>(null);
  const inspected = useRef(false);
  const following = useRef(true);
  useEffect(() => { if (!inspected.current) setOpen(live); }, [live]);
  useEffect(() => {
    if (live && open && following.current && content.current) content.current.scrollTop = content.current.scrollHeight;
  }, [text, live, open]);
  const measured = typeof durationMs === 'number' && Number.isFinite(durationMs) && durationMs >= 0;
  const seconds = measured ? Math.round(durationMs / 1000) : 0;
  const duration = seconds < 5 ? 'a few seconds' : seconds < 60 ? `${seconds} seconds` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return <details className="trace-thought-details" data-testid="turn-reasoning" open={open} onToggle={e => setOpen(e.currentTarget.open)}>
    <summary className="card-header trace-thought-pill" onClick={() => { inspected.current = true; }}><TraceIcon kind="thought" /><span className={`trace-thought-label${thinking ? ' is-live' : ''}`}>{thinking ? 'Thinking…' : measured ? `Thought · ${duration}` : 'Thought'}</span><ChevronRight size={14} className="trace-chevron thought-chevron" aria-hidden="true" /></summary>
    <div className="trace-thought-content" ref={content} onScroll={event => { const node = event.currentTarget; following.current = node.scrollHeight - node.scrollTop - node.clientHeight < 24; }}><MarkdownView content={text} assistant streaming={thinking} /></div>
  </details>;
}

export function ActivityTimeline({ events, messages, renderMessage, renderTool, generating = false }: {
  events: SessionEvent[]; messages: Message[];
  renderMessage: (message: Message) => ReactNode; renderTool: (message: Message) => ReactNode;
  generating?: boolean;
}) {
  const assistants = messages.filter(m => m.role === 'assistant');
  const assistant = assistants[assistants.length - 1];
  const toolEntries = useMemo(() => activityToolEntries(events, messages), [events, messages]);
  // Recovery and notification batches may overlap or arrive out of order.
  const ordered = useMemo(() => [...new Map(withLiveRounds(events, assistant).map(event => [event.seq, event])).values()].sort((a, b) => a.seq - b.seq), [events, assistant]);
  const thoughts = ordered.filter(event => event.eventType === 'reasoning' && typeof (event.payload as { text?: unknown }).text === 'string' && (event.payload as { text: string }).text.length);
  const recordedReasoning = thoughts.map(event => (event.payload as { text: string }).text).join('');
  const reasoning = assistant?.reasoning || recordedReasoning;
  // Exact matching avoids duplicating text from legacy or incomplete journals.
  const canSplitThoughts = Boolean(thoughts.length && reasoning.startsWith(recordedReasoning));
  const compactions = ordered.filter(event => event.eventType === 'compaction');
  const separated = Boolean(assistant && ordered.some(event => event.eventType === 'answer_committed' &&
    (event.payload as { messageId?: string; content?: string; format?: string }).messageId === assistant.id &&
    (event.payload as { content?: string; format?: string }).format === 'final_only' &&
    (event.payload as { content?: string }).content === assistant.content));
  const segments = assistant && assistants.length === 1 && assistant.presentation?.format !== 'openui' ? activitySegments(ordered, assistant.content, separated) : [];
  let remaining = assistant?.content ?? '';
  const groups: ReactNode[] = [];
  type Entry = { seq: number; node: ReactNode };
  const entries: Entry[] = [];
  let thoughtIndex = 0;
  if (canSplitThoughts) for (const event of thoughts) {
    const payload = event.payload as { text: string; durationMs?: number; live?: boolean };
    const key = event.runId && event.stepId ? `thought-${JSON.stringify([event.runId, event.stepId])}` : `thought-${thoughtIndex}`;
    thoughtIndex++;
    entries.push({ seq: event.seq, node: <Thought key={key} text={payload.text} durationMs={payload.durationMs} live={payload.live} thinking={payload.live && !assistant?.liveRounds?.find(round => round.runId === event.runId && round.stepId === event.stepId)?.content.trim()} /> });
  }
  else if (reasoning) {
    const duration = thoughts.length === 1 && recordedReasoning === reasoning ? (thoughts[0].payload as { durationMs?: number }).durationMs : undefined;
    groups.push(<Thought key="thought-0" text={reasoning} durationMs={duration} live={generating} thinking={generating && !assistant?.content.trim()} />);
  }
  for (const segment of segments) {
    entries.push({ seq: segment.seq, node: <div key={`commentary-${segment.seq}`} className="activity-commentary">{renderMessage({ ...assistant!, id: `${assistant!.id}-round-${segment.seq}`, content: segment.content, reasoning: '', status: 'complete', presentation: undefined, metrics: null, error: null })}</div> });
    if (!separated) remaining = remaining.trimStart().slice(segment.content.length);
  }
  let lastSeq = -Infinity;
  for (const entry of entries.sort((a, b) => a.seq - b.seq)) {
    const before = toolEntries.filter(tool => tool.seq > lastSeq && tool.seq < entry.seq).map(tool => tool.message);
    if (before.length) groups.push(<ActivityGroup key={`tools-${toolRowKey(before[0])}`} tools={before} renderTool={renderTool} live={false} />);
    groups.push(entry.node);
    lastSeq = entry.seq;
  }
  const trailing = toolEntries.filter(tool => tool.seq > lastSeq).map(tool => tool.message);
  if (trailing.length) groups.push(<ActivityGroup key={`tools-${toolRowKey(trailing[0])}`} tools={trailing} renderTool={renderTool} live={generating} />);
  if (canSplitThoughts && reasoning.length > recordedReasoning.length) {
    groups.push(<Thought key={`thought-${thoughtIndex}`} text={reasoning.slice(recordedReasoning.length)} live={generating} thinking={generating} />);
  }
  return <div className="activity-feed">
    {groups}
    {assistants.map(message => <Fragment key={message.id}>{renderMessage(message === assistant && segments.length ? { ...message, content: remaining.trimStart() } : message)}</Fragment>)}
    {compactions.map(event => <details className="activity-compaction" key={event.id}><summary>Context compacted · continuing</summary><p>{typeof (event.payload as { artifact?: unknown }).artifact === 'string' ? (event.payload as { artifact: string }).artifact : 'Older activity was summarized to make room for the next steps.'}</p></details>)}
  </div>;
}
