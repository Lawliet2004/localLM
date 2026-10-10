import { Fragment, useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { Message, RunRecord, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { TraceIcon, groupKindOf, toolKindOf, type TraceKind } from './TraceIcon';
import { WorkSummary } from './WorkSummary';

export function useSessionActivity(conversationId: string, generating: boolean) {
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const position = useRef({ conversationId, cursor: 0 });
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    if (position.current.conversationId !== conversationId) {
      position.current = { conversationId, cursor: 0 };
      setEvents([]);
    }
    const currentPosition = position.current;
    if (!nativeAvailable || conversationId === 'new' || !api.getSessionEvents) return;
    async function load() {
      try {
        const batch = await api.getSessionEvents(conversationId, currentPosition.cursor, 500);
        if (disposed) return;
        if (batch.length) {
          // Store::session_events returns unique rows ordered by seq, starting
          // at the cursor; only the new page needs to be appended.
          currentPosition.cursor = batch[batch.length - 1].seq + 1;
          setEvents(current => [...current, ...batch]);
        }
        if (batch.length === 500 || generating) timer = setTimeout(() => void load(), batch.length === 500 ? 0 : 750);
      } catch { if (!disposed && generating) timer = setTimeout(() => void load(), 2000); }
    }
    void load();
    return () => { disposed = true; clearTimeout(timer); };
  }, [conversationId, generating]);
  return events;
}

interface TimelineItem {
  id: string;
  kind: 'model_response' | 'compaction' | 'edit' | 'tool' | 'thought';
  icon?: TraceKind;
  toolMessage?: Message;
  content?: string;
  artifact?: string;
  thoughtText?: string;
  duration?: string;
  isFolder?: boolean;
  isSearch?: boolean;
  isTask?: boolean;
  isCommand?: boolean;
}

interface ActivityBlock {
  key: string;
  type: 'group' | 'edit' | 'compaction' | 'model_response';
  items?: TimelineItem[];
  item?: TimelineItem;
  title?: string;
  icon?: TraceKind;
}

function toolItem(id: string, toolName: string, args: Record<string, any>, result: Record<string, any>, toolMessage: Message): TimelineItem {
  return {
    id,
    kind: /edit|create_file|write_to_file|replace/.test(toolName) || result?.diff ? 'edit' : 'tool',
    icon: toolKindOf(toolName, args, result),
    toolMessage,
    isSearch: /search|grep|find|locate/.test(toolName),
    isFolder: /list_dir|list_files|browse_dir/.test(toolName),
    isTask: toolName === 'manage_task',
    isCommand: Boolean(args.command || args.CommandLine || /exec|run_code|terminal|run_command/.test(toolName)),
  };
}

export function ActivityTimeline({ events, messages, renderMessage, renderTool, compact = false, run }: {
  events: SessionEvent[]; messages: Message[];
  /** `final` is true only for the answer shown outside the work disclosure. */
  renderMessage: (message: Message, final?: boolean) => ReactNode; renderTool: (message: Message) => ReactNode;
  compact?: boolean; run?: RunRecord | null;
}) {
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});

  const assistant = [...messages].reverse().find(m => m.role === 'assistant');
  const base: Message = assistant ?? { id: '', conversationId: '', role: 'assistant', content: '', reasoning: '', createdAt: 0, status: 'complete' };
  const completedText = events.filter(e => e.eventType === 'model_response').map(e => (e.payload as {content?: string}).content ?? '').join('');
  const completedReasoning = events.filter(e => e.eventType === 'reasoning').map(e => (e.payload as {text?: string}).text ?? '').join('');
  const liveText = assistant?.content.startsWith(completedText) ? assistant.content.slice(completedText.length) : '';
  const liveReasoning = assistant?.reasoning.startsWith(completedReasoning) ? assistant.reasoning.slice(completedReasoning.length) : '';
  const isStreaming = assistant?.status === 'streaming';

  // Parse events into typed TimelineItems
  const timelineItems: TimelineItem[] = [];

  if (events.length === 0) {
    if (assistant?.reasoning) {
      timelineItems.push({
        id: `thought-${assistant.id || 'initial'}`,
        kind: 'thought',
        thoughtText: assistant.reasoning,
      });
    }

    for (const msg of messages) {
      if (msg.role === 'tool') {
        let toolName = 'tool';
        let args: Record<string, any> = {};
        let res: Record<string, any> = {};
        try {
          const parsed = JSON.parse(msg.content);
          const req = parsed.request || parsed;
          toolName = req.name || 'tool';
          args = req.arguments || {};
          res = parsed.result || {};
        } catch { /* ignore */ }
        if (isPlanStateTool(toolName)) continue;

        timelineItems.push(toolItem(msg.id, toolName, args, res, msg));
      }
    }

    if (assistant?.content && !isStreaming) {
      timelineItems.push({
        id: `resp-${assistant.id || 'final'}`,
        kind: 'model_response',
        content: assistant.content,
      });
    }
  } else {
    const callKey = (event: SessionEvent) => JSON.stringify([event.runId, event.toolCallId]);
    const results = new Map<string, Record<string, any>>();
    const endedRuns = new Set<string | null | undefined>();
    const liveMessages = new Map(messages.map(message => [message.id, message]));
    for (const event of events) {
      if (event.eventType === 'tool_result' && !results.has(callKey(event))) results.set(callKey(event), event.payload as Record<string, any>);
      if (event.eventType === 'turn_end') endedRuns.add(event.runId);
    }
    for (let i = 0; i < events.length; i++) {
      const event = events[i];
      const payload = (event.payload ?? {}) as Record<string, any>;

      if (event.eventType === 'model_response') {
        timelineItems.push({
          id: event.id,
          kind: 'model_response',
          content: payload.content ?? '',
        });
        continue;
      }

      if (event.eventType === 'compaction') {
        timelineItems.push({
          id: event.id,
          kind: 'compaction',
          artifact: payload.artifact,
        });
        continue;
      }

      if (event.eventType === 'reasoning') {
        let durationStr: string | undefined = undefined;
        if (typeof payload.duration === 'string') {
          durationStr = payload.duration;
        } else if (typeof payload.durationMs === 'number') {
          durationStr = `${Math.round(payload.durationMs / 1000)}s`;
        } else if (typeof payload.seconds === 'number') {
          durationStr = `${payload.seconds}s`;
        } else if (events[i + 1]?.createdAt && event.createdAt) {
          const delta = events[i + 1].createdAt - event.createdAt;
          const sec = Math.round(delta / 1000);
          if (sec > 0 && sec < 3600) {
            durationStr = `${sec}s`;
          }
        }

        timelineItems.push({
          id: event.id,
          kind: 'thought',
          thoughtText: payload.text ?? '',
          duration: durationStr,
        });
        continue;
      }

      if (event.eventType === 'tool_call') {
        const callName = String((payload as { name?: string }).name ?? '');
        if (isPlanStateTool(callName)) continue;
        const result = results.get(callKey(event));
        const live = liveMessages.get(`tool-${event.toolCallId}`);
        const ended = endedRuns.has(event.runId);
        const toolMessage: Message = result ? {
          ...base, id: event.id, role: 'tool', status: 'complete',
          content: JSON.stringify({ request: { name: result.name ?? payload.name, arguments: payload.arguments, decision: result.decision }, result: result.result }),
        } : live ?? { ...base, id: event.id, role: 'tool', status: ended ? 'interrupted' : 'streaming', content: JSON.stringify({ request: { name: payload.name, arguments: payload.arguments, decision: ended ? undefined : 'pending' }, result: {} }) };

        const toolName = (result?.name ?? payload.name ?? '') as string;
        const args = (payload.arguments ?? {}) as Record<string, any>;
        const res = (result?.result ?? {}) as Record<string, any>;
        timelineItems.push(toolItem(event.id, toolName, args, res, toolMessage));
        continue;
      }
    }
  }

  // Group items into blocks
  const blocks: ActivityBlock[] = [];
  let pendingGroup: TimelineItem[] = [];

  const flushPendingGroup = (isActiveEnd = false) => {
    if (!pendingGroup.length) return;
    const items = [...pendingGroup];
    const key = `grp-${items[0].id}`;

    // Compute title
    const fileCount = items.filter(it => it.kind === 'tool' && !it.isSearch && !it.isFolder && !it.isTask && !it.isCommand).length;
    const folderCount = items.filter(it => it.isFolder).length;
    const searchCount = items.filter(it => it.isSearch).length;
    const taskCount = items.filter(it => it.isTask).length;
    const commandCount = items.filter(it => it.isCommand).length;

    let title = '';
    if (commandCount > 0 && fileCount === 0 && searchCount === 0 && folderCount === 0 && taskCount === 0) {
      const prefix = isActiveEnd ? 'Running' : 'Ran';
      title = `${prefix} ${commandCount} ${commandCount === 1 ? 'command' : 'commands'}`;
    } else {
      const parts: string[] = [];
      if (fileCount > 0) parts.push(`${fileCount} ${fileCount === 1 ? 'file' : 'files'}`);
      if (folderCount > 0) parts.push(`${folderCount} ${folderCount === 1 ? 'folder' : 'folders'}`);
      if (searchCount > 0) parts.push(`${searchCount} ${searchCount === 1 ? 'search' : 'searches'}`);
      if (taskCount > 0) parts.push(`${taskCount} ${taskCount === 1 ? 'task' : 'tasks'}`);
      if (!parts.length && commandCount > 0) parts.push(`${commandCount} ${commandCount === 1 ? 'command' : 'commands'}`);
      if (!parts.length && items.some(it => it.kind === 'thought')) parts.push(`${items.length} ${items.length === 1 ? 'item' : 'items'}`);
      const prefix = isActiveEnd ? 'Exploring' : 'Explored';
      title = `${prefix} ${parts.join(', ')}`;
    }

    const icon = items.every(it => it.kind === 'thought')
      ? 'thought'
      : groupKindOf(items.flatMap(it => (it.icon ? [it.icon] : [])));

    blocks.push({
      key,
      type: 'group',
      items,
      title,
      icon,
    });
    pendingGroup = [];
  };

  for (let idx = 0; idx < timelineItems.length; idx++) {
    const item = timelineItems[idx];
    if (item.kind === 'model_response') {
      flushPendingGroup(false);
      blocks.push({ key: `resp-${item.id}`, type: 'model_response', item });
    } else if (item.kind === 'compaction') {
      flushPendingGroup(false);
      blocks.push({ key: `comp-${item.id}`, type: 'compaction', item });
    } else if (item.kind === 'edit') {
      flushPendingGroup(false);
      blocks.push({ key: `edit-${item.id}`, type: 'edit', item });
    } else {
      pendingGroup.push(item);
    }
  }
  flushPendingGroup(isStreaming);

  const finalBlockIndex = compact && !isStreaming ? blocks.map(block => block.type).lastIndexOf('model_response') : -1;
  const finalBlock = finalBlockIndex >= 0 ? blocks[finalBlockIndex] : null;
  const displayBlocks = finalBlockIndex >= 0 ? blocks.filter((_, index) => index !== finalBlockIndex) : blocks;
  const extraText = events.length > 0 && assistant?.content.startsWith(completedText) ? assistant.content.slice(completedText.length) : '';
  const finalText = finalBlock?.item ? `${finalBlock.item.content ?? ''}${extraText}` : assistant?.content ?? '';
  const durationMs = events.length > 1 ? events[events.length - 1].createdAt - events[0].createdAt : null;

  const activity = (
    <div className="activity-feed">
      {displayBlocks.map((block, blockIndex) => {
        if (block.type === 'model_response' && block.item) {
          return (
            <Fragment key={block.key}>
              {renderMessage({
                ...base,
                id: block.item.id,
                content: block.item.content ?? '',
                reasoning: '',
                status: 'complete',
              })}
            </Fragment>
          );
        }

        if (block.type === 'compaction' && block.item) {
          return (
            <details key={block.key} className="activity-reasoning activity-compaction">
              <summary>
                <TraceIcon kind="system" size={12} />
                <span>Context compacted · continuing</span>
              </summary>
              <p>Older activity archived at the 80% threshold. Artifact: {block.item.artifact}</p>
            </details>
          );
        }

        if (block.type === 'edit' && block.item?.toolMessage) {
          return (
            <div key={block.key} className="activity-edit-wrapper">
              {renderTool(block.item.toolMessage)}
            </div>
          );
        }

        if (block.type === 'group' && block.items) {
          const isLastGroup = blockIndex === displayBlocks.length - 1;
          const isLive = isStreaming && isLastGroup;
          const defaultExpanded = isStreaming || isLastGroup || block.items.length <= 1;
          const isExpanded = openGroups[block.key] ?? defaultExpanded;

          return (
            <div key={block.key} className={`activity-group ${isLive ? 'is-live' : ''}`}>
              <button
                type="button"
                className={`activity-group-header ${isExpanded ? 'expanded' : ''}`}
                onClick={() => setOpenGroups(curr => ({ ...curr, [block.key]: !isExpanded }))}
                aria-expanded={isExpanded}
              >
                <TraceIcon kind={block.icon ?? 'explore'} size={13} />
                <span className="activity-group-title">{block.title}</span>
                <ChevronRight size={13} className="activity-group-chevron" />
              </button>
              <div
                className="activity-group-items"
                style={{ display: isExpanded ? 'flex' : 'none' }}
              >
                {block.items.map(item => {
                  if (item.kind === 'thought') {
                    return (
                      <details key={item.id} className="trace-thought-details">
                        <summary className="trace-thought-pill">
                          <TraceIcon kind="thought" size={12} />
                          <span className="trace-thought-label">{item.duration ? `Thought for ${item.duration}` : 'Thought'}</span>
                          <ChevronRight size={13} className="thought-chevron" />
                        </summary>
                        <div className="trace-thought-content">{item.thoughtText}</div>
                      </details>
                    );
                  }
                  if (item.toolMessage) {
                    return (
                      <Fragment key={item.id}>
                        {renderTool(item.toolMessage)}
                      </Fragment>
                    );
                  }
                  return null;
                })}
              </div>
            </div>
          );
        }

        return null;
      })}

      {!compact && ((events.length > 0 && (liveText || liveReasoning || assistant?.status === 'streaming')) ||
        (events.length === 0 && isStreaming && (assistant?.content || assistant?.reasoning))) &&
        renderMessage({ ...base, id: `${base.id}-live`, content: events.length > 0 ? liveText : (assistant?.content ?? ''), reasoning: events.length > 0 ? liveReasoning : (assistant?.reasoning ?? '') })}
    </div>
  );
  if (!compact) return <>{activity}{assistant?.error && <p role="alert" className="error-banner">{assistant.error}</p>}</>;

  const steps = timelineItems.filter(item => item.kind !== 'model_response').length;
  return <WorkSummary messages={messages} run={run} durationMs={durationMs} running={isStreaming} initiallyOpen={!assistant} steps={steps}
    activity={displayBlocks.length > 0 || liveReasoning ? activity : undefined}>
    {(finalText || (isStreaming && liveText) || assistant?.status === 'error' || assistant?.status === 'interrupted') && renderMessage({
      ...base, id: finalBlock?.item?.id ?? `${base.id}-live`,
      content: isStreaming ? liveText : finalText,
      reasoning: '',
    }, !isStreaming)}
  </WorkSummary>;
}
