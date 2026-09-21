import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { Message, SessionEvent } from '../lib/types';
import { isPlanStateTool } from './PlanChecklist';
import { TraceIcon, groupKindOf, toolKindOf, type TraceKind } from './TraceIcon';

export function useSessionActivity(conversationId: string, generating: boolean) {
  const [events, setEvents] = useState<SessionEvent[]>([]);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    let cursor = 0;
    setEvents([]);
    if (!nativeAvailable || conversationId === 'new' || !api.getSessionEvents) return;
    async function load() {
      try {
        const batch = await api.getSessionEvents(conversationId, cursor, 500);
        if (disposed) return;
        if (batch.length) {
          cursor = Math.max(...batch.map(e => e.seq)) + 1;
          setEvents(current => [...new Map([...current, ...batch].map(e => [e.seq, e])).values()].sort((a, b) => a.seq - b.seq));
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
  rawEvent?: SessionEvent;
  toolMessage?: Message;
  content?: string;
  artifact?: string;
  thoughtText?: string;
  duration?: string;
  toolName?: string;
  filename?: string;
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

export function ActivityTimeline({ events, messages, renderMessage, renderTool }: {
  events: SessionEvent[]; messages: Message[];
  renderMessage: (message: Message) => ReactNode; renderTool: (message: Message) => ReactNode;
}) {
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});

  const assistant = messages.find(m => m.role === 'assistant');
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
        duration: isStreaming ? undefined : '8s',
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

        const diff = res?.diff;
        const path = (args.path ?? args.AbsolutePath ?? args.TargetFile ?? args.file) as string | undefined;
        const filename = path ? path.split(/[\\/]/).pop() : undefined;
        const isEdit = /edit|create_file|write_to_file|replace/.test(toolName) || Boolean(diff);
        const isSearch = /search|grep|find|locate/.test(toolName);
        const isFolder = /list_dir|list_files|browse_dir/.test(toolName);
        const isTask = toolName === 'manage_task';
        const isCommand = Boolean(args.command || args.CommandLine || /exec|run_code|terminal|run_command/.test(toolName));

        timelineItems.push({
          id: msg.id,
          kind: isEdit ? 'edit' : 'tool',
          icon: toolKindOf(toolName, args, res),
          toolMessage: msg,
          toolName,
          filename,
          isFolder,
          isSearch,
          isTask,
          isCommand,
        });
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
    for (let i = 0; i < events.length; i++) {
      const event = events[i];
      const payload = (event.payload ?? {}) as Record<string, any>;

      if (event.eventType === 'model_response') {
        timelineItems.push({
          id: event.id,
          kind: 'model_response',
          rawEvent: event,
          content: payload.content ?? '',
        });
        continue;
      }

      if (event.eventType === 'compaction') {
        timelineItems.push({
          id: event.id,
          kind: 'compaction',
          rawEvent: event,
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
          const sec = delta > 10000 ? Math.round(delta / 1000) : delta;
          if (sec > 0 && sec < 3600) {
            durationStr = `${sec}s`;
          }
        }
        if (!durationStr) {
          const words = (payload.text ?? '').trim().split(/\s+/).filter(Boolean).length;
          const estimatedSec = Math.max(2, Math.min(30, Math.round(words / 15)));
          durationStr = `${estimatedSec}s`;
        }

        timelineItems.push({
          id: event.id,
          kind: 'thought',
          rawEvent: event,
          thoughtText: payload.text ?? '',
          duration: durationStr,
        });
        continue;
      }

      if (event.eventType === 'tool_call') {
        const callName = String((payload as { name?: string }).name ?? '');
        if (isPlanStateTool(callName)) continue;
        const result = events.find(e => e.eventType === 'tool_result' && e.runId === event.runId && e.toolCallId === event.toolCallId)?.payload as Record<string, any> | undefined;
        const live = messages.find(m => m.id === `tool-${event.toolCallId}`);
        const ended = events.some(e => e.eventType === 'turn_end' && e.runId === event.runId);
        const toolMessage: Message = result ? {
          ...base, id: event.id, role: 'tool', status: 'complete',
          content: JSON.stringify({ request: { name: result.name ?? payload.name, arguments: payload.arguments, decision: result.decision }, result: result.result }),
        } : live ?? { ...base, id: event.id, role: 'tool', status: ended ? 'interrupted' : 'streaming', content: JSON.stringify({ request: { name: payload.name, arguments: payload.arguments, decision: ended ? undefined : 'pending' }, result: {} }) };

        const toolName = (result?.name ?? payload.name ?? '') as string;
        const args = (payload.arguments ?? {}) as Record<string, any>;
        const res = (result?.result ?? {}) as Record<string, any>;
        const diff = res?.diff;
        const path = (args.path ?? args.AbsolutePath ?? args.TargetFile ?? args.file) as string | undefined;
        const filename = path ? path.split(/[\\/]/).pop() : undefined;

        const isEdit = /edit|create_file|write_to_file|replace/.test(toolName) || Boolean(diff);
        const isSearch = /search|grep|find|locate/.test(toolName);
        const isFolder = /list_dir|list_files|browse_dir/.test(toolName);
        const isTask = toolName === 'manage_task';
        const isCommand = Boolean(args.command || args.CommandLine || /exec|run_code|terminal|run_command/.test(toolName));

        timelineItems.push({
          id: event.id,
          kind: isEdit ? 'edit' : 'tool',
          icon: toolKindOf(toolName, args, res),
          rawEvent: event,
          toolMessage,
          toolName,
          filename,
          isFolder,
          isSearch,
          isTask,
          isCommand,
        });
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

  return (
    <div className="activity-feed">
      {blocks.map((block, blockIndex) => {
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
          const isLastGroup = blockIndex === blocks.length - 1;
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
                <span className="activity-group-count">{block.items.length}</span>
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
                          <span className="trace-thought-label">Thought for {item.duration ?? '8s'}</span>
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

      {((events.length > 0 && (liveText || liveReasoning || assistant?.status === 'streaming')) ||
        (events.length === 0 && isStreaming && (assistant?.content || assistant?.reasoning))) &&
        renderMessage({ ...base, id: `${base.id}-live`, content: events.length > 0 ? liveText : (assistant?.content ?? ''), reasoning: events.length > 0 ? liveReasoning : (assistant?.reasoning ?? '') })}
      {assistant?.error && <p role="alert" className="error-banner">{assistant.error}</p>}
    </div>
  );
}
