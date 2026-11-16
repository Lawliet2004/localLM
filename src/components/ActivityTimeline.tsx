import { toolRecord, toolOperationId, toolFailed } from '../lib/toolActivity';
import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { Message, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { ReasoningDisclosure } from './ReasoningDisclosure';
import { TraceIcon, groupKindOf, toolKindOf } from './TraceIcon';
import { reduceTurnItems, subscribeTurnItems, type TurnItem } from '../lib/turnItems';

export function useSessionActivity(conversationId: string, generating: boolean) {
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [hasEarlier, setHasEarlier] = useState(false);
  const [loadingEarlier, setLoadingEarlier] = useState(false);
  const [historyError, setHistoryError] = useState('');
  const generatingRef = useRef(generating);
  generatingRef.current = generating;
  const previousGenerating = useRef(generating);
  const reload = useRef<(() => void) | null>(null);
  const loadEarlier = useRef<(() => void) | null>(null);
  useEffect(() => {
    let disposed = false, loading = false, queued = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cursor = 0;
    let initial = true;
    let earlier: number | undefined;
    let readingEarlier = false;
    setEvents([]);
    setHasEarlier(false);
    setLoadingEarlier(false);
    setHistoryError('');
    if (!nativeAvailable || conversationId === 'new' || !api.getSessionEvents) return;
    const pageSize = 500;
    const merge = (batch: SessionEvent[]) => setEvents(current => [...new Map([...current, ...batch].map(event => [event.seq, event])).values()].sort((a, b) => a.seq - b.seq));
    const receiveEarlier = (batch: SessionEvent[]) => {
      const first = batch[0]?.seq;
      earlier = batch.length === pageSize && first > 1 ? first : undefined;
      setHasEarlier(earlier !== undefined);
    };
    loadEarlier.current = async () => {
      if (disposed || readingEarlier || earlier === undefined) return;
      readingEarlier = true;
      setLoadingEarlier(true);
      setHistoryError('');
      try {
        const batch = await api.getSessionEvents(conversationId, null, pageSize, { beforeSeq: earlier });
        if (disposed) return;
        receiveEarlier(batch);
        if (batch.length) merge(batch);
      } catch {
        if (!disposed) setHistoryError('Earlier activity could not be loaded.');
      } finally {
        readingEarlier = false;
        if (!disposed) setLoadingEarlier(false);
      }
    };
    async function load() {
      if (disposed) return;
      if (loading) { queued = true; return; }
      loading = true; clearTimeout(timer);
      let full = false, failed = false;
      try {
        const batch = initial ? await api.getSessionEvents(conversationId, null, pageSize, { latest: true })
          : await api.getSessionEvents(conversationId, cursor, pageSize);
        if (disposed) return;
        if (initial) receiveEarlier(batch);
        full = !initial && batch.length === pageSize;
        initial = false;
        if (batch.length) {
          cursor = Math.max(cursor, batch[batch.length - 1].seq + 1);
          merge(batch);
        }
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
    const stopItems = subscribeTurnItems(conversationId, event => {
      if (disposed) return;
      setEvents(current => current.some(existing => existing.seq === event.seq) ? current : [...current, event].sort((a, b) => a.seq - b.seq));
      const item = (event.payload as { item?: { kind?: unknown } } | null)?.item;
      if (event.eventType === 'item.started' && item?.kind === 'approval') void load();
    });
    let unlisten: (() => void) | undefined;
    // Subscribe before recovery loading. Both paths read the same committed
    // journal; polling also covers missed notifications and older backends.
    if (api.onSessionActivity) void api.onSessionActivity(conversationId, seq => {
      if (!disposed && seq >= cursor) void load();
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => { /* Polling remains available. */ });
    void load();
    return () => { disposed = true; stopItems(); unlisten?.(); clearTimeout(timer); reload.current = null; loadEarlier.current = null; };
  }, [conversationId]);
  useEffect(() => {
    if (previousGenerating.current === generating) return;
    previousGenerating.current = generating;
    reload.current?.();
  }, [generating]);
  return { events, hasEarlier, loadingEarlier, historyError, loadEarlier: () => loadEarlier.current?.() };
}

/** Duplicate delivery must not duplicate an operation or move it in the log. */
function toolCalls(events: SessionEvent[]) {
  const seen = new Set<string>();
  return [...events].sort((a, b) => a.seq - b.seq).filter(event => {
    if (event.eventType !== 'tool_call' || isPlanStateTool(String((event.payload as { name?: string }).name ?? ''))) return false;
    const key = JSON.stringify([event.runId, event.stepId, event.toolCallId ?? event.id]);
    if (seen.has(key)) return false;
    seen.add(key); return true;
  });
}

const replay = new WeakMap<SessionEvent, { result?: SessionEvent; processId?: string; ended: boolean; awaiting: boolean; interruptedApproval: boolean; message: Message }>();

/** Run identity and call intervals prevent reused provider IDs crossing runs.
 * Immutable event objects also let completed replay rows retain their identity. */
export function activityTools(events: SessionEvent[], messages: Message[], items = reduceTurnItems(events), active = true): Message[] {
  const calls = toolCalls(events);
  if (!calls.length) return messages.filter(message => message.role === 'tool' && !isPlanMessage(message));
  const resultEvents = [...events].sort((a, b) => a.seq - b.seq).filter(event => event.eventType === 'tool_result');
  const results = new Map(resultEvents.map(event => [JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId]), event]));
  const processes = new Map(events.filter(event => event.eventType === 'process_started' && event.toolCallId)
    .map(event => [JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId]), (event.payload as { processId?: string })?.processId]));
  const pending = new Set(items.filter(item => item.kind === 'approval' && item.status === 'awaiting' && typeof item.payload.callId === 'string')
    .map(item => JSON.stringify([item.runId, item.payload.stepId ?? null, item.payload.callId])));
  const interruptedApprovals = new Set(items.filter(item => item.kind === 'approval' && item.status === 'interrupted' && typeof item.payload.callId === 'string')
    .map(item => JSON.stringify([item.runId, item.payload.stepId ?? null, item.payload.callId])));
  const ended = new Set(events.filter(event => event.eventType === 'turn_end').map(event => event.runId));
  const byMessageId = new Map<string, Message>();
  const candidates = new Map<string, Message[]>();
  const legacy = new Map<string, Message[]>();
  for (const message of messages) if (message.role === 'tool') {
    byMessageId.set(message.id, message);
    const request = toolRecord(message).request;
    for (const key of new Set([message.id, typeof request?.callId === 'string' ? `tool-${request.callId}` : message.id])) {
      const list = candidates.get(key) ?? []; list.push(message); candidates.set(key, list);
    }
    if (request && !request.callId) {
      const key = JSON.stringify([request.name, request.arguments ?? {}]);
      const list = legacy.get(key) ?? []; list.push(message); legacy.set(key, list);
    }
  }
  const lastCallById = new Map<string | null | undefined, SessionEvent>();
  const nextCalls = new Map<SessionEvent, SessionEvent>();
  const reused = new Set<string | null | undefined>();
  for (const call of calls) {
    const previous = lastCallById.get(call.toolCallId);
    if (previous) { nextCalls.set(previous, call); reused.add(call.toolCallId); }
    lastCallById.set(call.toolCallId, call);
  }
  const matched = new Set<Message>();
  const projected = calls.map((event): Message => {
    const next = nextCalls.get(event);
    const resultEvent = results.get(JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId]));
    const resultMessageId = (resultEvent?.payload as { messageId?: string } | undefined)?.messageId;
    const call = event.payload as Record<string, any>;
    const persisted = (resultMessageId ? byMessageId.get(resultMessageId) : undefined) ?? candidates.get(`tool-${event.toolCallId}`)?.find(message => {
      if (matched.has(message)) return false;
      const request = toolRecord(message).request;
      const runId = request?.runId;
      if (request?.stepId) return (!runId || runId === event.runId) && request.stepId === event.stepId;
      if (runId && runId !== event.runId) return false;
      return !reused.has(event.toolCallId) || message.createdAt >= event.createdAt && (!next || message.createdAt < next.createdAt);
    }) ?? legacy.get(JSON.stringify([call.name, call.arguments ?? {}]))?.find(message => {
      const runId = toolRecord(message).request?.runId;
      return !matched.has(message) && (!runId || runId === event.runId);
    });
    if (persisted) matched.add(persisted);
    const finished = ended.has(event.runId);
    const approvalKey = JSON.stringify([event.runId, event.stepId ?? null, event.toolCallId]);
    const hasPendingApproval = !resultEvent && pending.has(approvalKey);
    const awaiting = !finished && active && hasPendingApproval;
    const interruptedApproval = !resultEvent && (interruptedApprovals.has(approvalKey) || hasPendingApproval && (finished || !active));
    const processId = !resultEvent && !finished && active ? processes.get(approvalKey) : undefined;
    const runningProcess = processId ? { data: { processId, processState: 'running' }, metadata: { family: 'harness' } } : undefined;
    if (persisted && (persisted.status !== 'streaming' || !resultEvent)) {
      if ((!awaiting && !interruptedApproval && !runningProcess) || persisted.status !== 'streaming') return persisted;
      const record = toolRecord(persisted);
      return { ...persisted, status: interruptedApproval ? 'interrupted' : persisted.status,
        content: JSON.stringify({ ...record, request: { ...(record.request ?? {}), decision: awaiting ? 'pending' : record.request?.decision },
          result: runningProcess ?? record.result }) };
    }
    const cached = replay.get(event);
    if (cached && cached.result === resultEvent && cached.processId === processId && cached.ended === finished && cached.awaiting === awaiting && cached.interruptedApproval === interruptedApproval) return cached.message;
    const result = resultEvent?.payload as Record<string, any> | undefined;
    const message: Message = {
      id: toolOperationId(event.toolCallId ?? event.id, event.runId, event.stepId), conversationId: event.conversationId, role: 'tool', reasoning: '', createdAt: event.createdAt,
      status: result ? 'complete' : finished || interruptedApproval ? 'interrupted' : 'streaming',
      content: JSON.stringify({ request: { ...call, callId: event.toolCallId, runId: event.runId, stepId: event.stepId, name: result?.name ?? call.name, arguments: call.arguments, decision: result?.decision ?? (awaiting ? 'pending' : finished ? undefined : call.decision) }, result: result ? result.result ?? result : runningProcess ?? {} }),
    };
    replay.set(event, { result: resultEvent, processId, ended: finished, awaiting, interruptedApproval, message });
    return message;
  });
  return [...projected, ...messages.filter(message => message.role === 'tool' && !matched.has(message) && !isPlanMessage(message))];
}

/** Unjournaled settlements retain a place in the same chronological feed. */
function sequencedTools(events: SessionEvent[], tools: Message[]): Map<number, Message> {
  const calls = toolCalls(events);
  const entries = new Map(calls.map((event, index) => [event.seq, tools[index]]));
  for (const message of tools.slice(calls.length)) {
    const next = events.find(event => event.createdAt > message.createdAt);
    let seq = next ? next.seq - 0.5 : (events[events.length - 1]?.seq ?? 0) + 0.5;
    while (entries.has(seq)) seq += 0.0001;
    entries.set(seq, message);
  }
  const owned = new Map<string, { seq: number; message: Message; frames: Map<number, unknown> }>();
  for (const [seq, message] of entries) {
    const record = toolRecord(message), request = record.request ?? {}, result = record.result ?? {}, data = result.data ?? result;
    if (!['process_start', 'shell_run', 'process_poll', 'process_write', 'process_stop'].includes(request.name)) continue;
    const id = typeof data.id === 'string' ? data.id : typeof data.processId === 'string' ? data.processId : undefined;
    if (!id) continue;
    const key = JSON.stringify([message.conversationId, id]);
    let prior = owned.get(key);
    if (!prior) { prior = { seq, message, frames: new Map() }; owned.set(key, prior); }
    for (const frame of Array.isArray(data.output) ? data.output : []) if (typeof frame.cursor === 'number') prior.frames.set(frame.cursor, frame);
    if (prior.message === message) continue;
    const first = toolRecord(prior.message), firstData = first.result?.data ?? first.result ?? {};
    const mergedData = { ...firstData, ...data, output: [...prior.frames.values()] };
    const merged = { ...prior.message, status: message.status, content: JSON.stringify({ ...first, result: result.data ? { ...result, data: mergedData } : mergedData }) };
    entries.set(prior.seq, merged);
    entries.delete(seq);
  }
  return entries;
}
function isPlanMessage(message: Message) {
  const value = toolRecord(message); return isPlanStateTool(value.request?.name ?? value.name ?? '');
}
function needsAttention(message: Message) {
  if (['streaming', 'error', 'interrupted'].includes(message.status)) return true;
  const value = toolRecord(message);
  return toolFailed(value) || value.request?.decision === 'pending';
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
function ActivityGroup({ tools, renderTool }: { tools: Message[]; renderTool: (message: Message) => ReactNode }) {
  const rows: ReactNode[] = [];
  let exploration: Message[] = [];
  const flush = () => {
    if (!exploration.length) return;
    const group = exploration;
    exploration = [];
    if (group.length === 1) { rows.push(<Fragment key={group[0].id}>{renderTool(group[0])}</Fragment>); return; }
    const kinds = group.map(tool => { const meta = toolMeta(tool); return toolKindOf(meta.name, meta.args); });
    const paths = group.map(tool => toolMeta(tool).args.path);
    const fileCount = new Set(paths).size;
    const label = paths.every(path => typeof path === 'string') && kinds.every(kind => kind === 'read')
      ? `Read ${fileCount} ${fileCount === 1 ? 'file' : 'files'}`
      : `${activityLabel(group)} · ${group.length} actions`;
    rows.push(<details className="activity-summary" key={group[0].id}>
      <summary className="card-header"><TraceIcon kind={groupKindOf(kinds)} size={14} /><span className="activity-summary-label">{label}</span><ChevronRight size={14} className="trace-chevron" aria-hidden="true" /></summary>
      <div className="activity-summary-rows">{group.map(tool => <Fragment key={tool.id}>{renderTool(tool)}</Fragment>)}</div>
    </details>);
  };
  for (const tool of tools) {
    const meta = toolMeta(tool);
    if (!needsAttention(tool) && ['read', 'search', 'folder', 'explore'].includes(toolKindOf(meta.name, meta.args))) exploration.push(tool);
    else { flush(); rows.push(<Fragment key={tool.id}>{renderTool(tool)}</Fragment>); }
  }
  flush();
  return <>{rows}</>;
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


function ToolDraftRow({ item, generating }: { item: TurnItem; generating: boolean }) {
  const args = item.payload.arguments && typeof item.payload.arguments === 'object' && !Array.isArray(item.payload.arguments)
    ? item.payload.arguments as Record<string, unknown> : {};
  const name = typeof item.payload.name === 'string' ? item.payload.name : '';
  const kind = toolKindOf(name, args);
  const target = typeof args.path === 'string' ? args.path : typeof args.command === 'string' ? args.command : name;
  const label = item.status === 'interrupted' ? 'Tool request incomplete · not run'
    : !generating ? 'Tool request interrupted · check activity'
    : kind === 'edit' ? 'Preparing edit request' : kind === 'command' ? 'Preparing command' : kind === 'read' ? 'Preparing read' : kind === 'search' ? 'Preparing search' : 'Preparing tool request';
  return <div className="activity-draft" aria-live="off"><TraceIcon kind={kind} size={14} /><span>{label}</span>{target && <code title={target}>{target.length > 160 ? `${target.slice(0, 160)}…` : target}</code>}</div>;
}

export function ActivityTimeline({ events, messages, renderMessage, renderTool, generating = false }: {
  events: SessionEvent[]; messages: Message[];
  renderMessage: (message: Message, variant?: 'commentary') => ReactNode; renderTool: (message: Message) => ReactNode;
  generating?: boolean;
}) {
  const assistants = messages.filter(m => m.role === 'assistant');
  const assistant = assistants[assistants.length - 1];
  const typedItems = useMemo(() => reduceTurnItems(events), [events]);
  const tools = useMemo(() => activityTools(events, messages, typedItems, generating), [events, messages, typedItems, generating]);
  // A turn can contain more than one assistant message (for example after a
  // retry). Prefer the saved/live message text to the journal, which records
  // the same reasoning by model round, and retain all messages in that turn.
  const messageReasoning = assistants.map(message => message.reasoning).filter(Boolean).join('\n\n');
  const reasoning = messageReasoning || events.filter(e => e.eventType === 'reasoning').map(e => (e.payload as { text?: string }).text ?? '').filter(Boolean).join('\n\n');
  const durations = events.filter(e => e.eventType === 'reasoning').map(e => (e.payload as { durationMs?: unknown }).durationMs);
  const measured = durations.length > 0 && durations.every(value => typeof value === 'number' && Number.isFinite(value) && value >= 0);
  const reasoningMs = measured ? (durations as number[]).reduce((sum, value) => sum + value, 0) : undefined;
  const compactions = events.filter(e => e.eventType === 'compaction');
  const hasAnswer = Boolean(assistant?.content?.trim());
  const live = generating && Boolean(reasoning);
  const thinking = live && !hasAnswer;
  const segments = assistant && assistants.length === 1 && assistant.presentation?.format !== 'openui' ? activitySegments(events, assistant.content) : [];
  let remaining = assistant?.content ?? '';

  // Only use per-round ordering when the journal is an exact prefix of the
  // persisted/live text. Older or partial journals otherwise risk omitting or
  // duplicating thought. Any unfinished current round remains a trailing card.
  const typedThoughts = useMemo(() => typedItems.filter(item => item.kind === 'reasoning' && typeof item.payload.text === 'string' && item.payload.text.length > 0)
    .map(item => ({ seq: item.seq, text: item.payload.text as string, durationMs: item.durationMs, status: item.status, thinking: item.status === 'running' && generating })), [typedItems, generating]);
  const drafts = typedItems.filter(item => item.kind === 'tool' && (item.status === 'running' || item.status === 'interrupted'));
  const legacyThoughts = [...new Map(events.filter(e => e.eventType === 'reasoning').map(e => [e.seq, e])).values()]
    .sort((a, b) => a.seq - b.seq)
    .flatMap(event => {
      if (!event.payload || typeof event.payload !== 'object' || Array.isArray(event.payload)) return [];
      const payload = event.payload as { text?: unknown; durationMs?: unknown };
      if (typeof payload.text !== 'string' || !payload.text) return [];
      const durationMs = typeof payload.durationMs === 'number' && Number.isFinite(payload.durationMs) && payload.durationMs >= 0 ? payload.durationMs : undefined;
      return [{ seq: event.seq, text: payload.text, durationMs, thinking: false }];
    });
  // A journal write can fail after a typed item started. Once the turn ends,
  // prefer a complete legacy round over an unterminated partial item.
  const typedIncomplete = !generating && typedThoughts.some(item => item.status === 'running');
  const usingTyped = typedThoughts.length > 0 && !(typedIncomplete && legacyThoughts.length);
  const recordedThoughts = usingTyped ? typedThoughts : legacyThoughts;
  const recordedText = recordedThoughts.map(thought => thought.text).join('');
  const orderedThoughts = assistants.length === 1 && recordedText && (reasoning.startsWith(recordedText) || (usingTyped && recordedText.startsWith(reasoning))) ? recordedThoughts : [];
  const feed: ReactNode[] = [];
  if (orderedThoughts.length) {
    const commentary = new Map(segments.map(segment => [segment.seq, segment]));
    let pendingThought = reasoning.startsWith(recordedText) ? reasoning.slice(recordedText.length) : '';
    const thoughtBySeq = new Map(orderedThoughts.map((thought, index) => {
      const appendLive = index === orderedThoughts.length - 1 && thought.thinking ? pendingThought : '';
      if (appendLive) pendingThought = '';
      return [thought.seq, { ...thought, text: thought.text + appendLive, index }];
    }));
    const toolBySeq = sequencedTools(events, tools);
    const draftBySeq = new Map(drafts.map(item => [item.seq, item]));
    const orderedSeqs = [...new Set([...commentary.keys(), ...thoughtBySeq.keys(), ...toolBySeq.keys(), ...draftBySeq.keys()])].sort((a, b) => a - b);
    let pendingTools: Message[] = [];
    const flushTools = () => {
      if (!pendingTools.length) return;
      feed.push(<ActivityGroup key={`tools-${pendingTools[0].id}`} tools={pendingTools} renderTool={renderTool} />);
      pendingTools = [];
    };
    for (const seq of orderedSeqs) {
      const tool = toolBySeq.get(seq);
      if (tool) { pendingTools.push(tool); continue; }
      flushTools();
      const thought = thoughtBySeq.get(seq);
      if (thought) feed.push(<ReasoningDisclosure key={`thought-${thought.index}`} text={thought.text} durationMs={thought.durationMs} active={thought.thinking} />);
      const draft = draftBySeq.get(seq);
      if (draft) feed.push(<ToolDraftRow key={draft.id} item={draft} generating={generating} />);
      const segment = commentary.get(seq);
      if (segment) {
        feed.push(<Fragment key={`commentary-${seq}`}>{renderMessage({ ...assistant!, id: `${assistant!.id}-round-${seq}`, content: segment.content, reasoning: '', status: 'complete', presentation: undefined, metrics: null, error: null }, 'commentary')}</Fragment>);
        remaining = remaining.trimStart().slice(segment.content.length);
      }
    }
    flushTools();
    // Message-only legacy tools have no sequence and remain visible.

    if (pendingThought) feed.push(<ReasoningDisclosure key={`thought-${orderedThoughts.length}`} text={pendingThought} active={generating} />);
  } else {
    if (reasoning) feed.push(<ReasoningDisclosure key="thought-0" text={reasoning} durationMs={reasoningMs} active={thinking} />);
    const commentary = new Map(segments.map(segment => [segment.seq, segment]));
    const toolBySeq = sequencedTools(events, tools);
    const draftBySeq = new Map(drafts.map(item => [item.seq, item]));
    const orderedSeqs = [...new Set([...commentary.keys(), ...toolBySeq.keys(), ...draftBySeq.keys()])].sort((a, b) => a - b);
    let pendingTools: Message[] = [];
    const flushTools = () => {
      if (!pendingTools.length) return;
      feed.push(<ActivityGroup key={`tools-${pendingTools[0].id}`} tools={pendingTools} renderTool={renderTool} />);
      pendingTools = [];
    };
    for (const seq of orderedSeqs) {
      const tool = toolBySeq.get(seq);
      if (tool) { pendingTools.push(tool); continue; }
      flushTools();
      const draft = draftBySeq.get(seq);
      if (draft) feed.push(<ToolDraftRow key={draft.id} item={draft} generating={generating} />);
      const segment = commentary.get(seq);
      if (segment) {
        feed.push(<Fragment key={`commentary-${seq}`}>{renderMessage({ ...assistant!, id: `${assistant!.id}-round-${seq}`, content: segment.content, reasoning: '', status: 'complete', presentation: undefined, metrics: null, error: null }, 'commentary')}</Fragment>);
        remaining = remaining.trimStart().slice(segment.content.length);
      }
    }
    flushTools();

  }
  return <div className="activity-feed">
    {feed}
    {assistants.map(message => <Fragment key={message.id}>{renderMessage(message === assistant && segments.length ? { ...message, content: remaining.trimStart() } : message)}</Fragment>)}
    {compactions.map(event => <details className="activity-compaction" key={event.id}><summary>Context compacted · continuing</summary><p>{typeof (event.payload as { artifact?: unknown }).artifact === 'string' ? (event.payload as { artifact: string }).artifact : 'Older activity was summarized to make room for the next steps.'}</p></details>)}
  </div>;
}
