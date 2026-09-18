import { useState, useEffect } from 'react';
import { GitFork, RefreshCw, AlertTriangle } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { SessionEvent } from '../lib/types';

export type EventCategory = 'all' | 'model' | 'tool' | 'system' | 'error';

function previewPayload(value: unknown): string {
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  const text = JSON.stringify(value, null, 2);
  return text.length > 8192 ? `${text.slice(0, 8192)}\n… (truncated for display)` : text;
}

function matchesCategory(event: SessionEvent, category: EventCategory): boolean {
  if (category === 'all') return true;

  const type = event.eventType.toLowerCase();
  const payload = event.payload as Record<string, unknown> | null;
  const hasError = type === 'error' || Boolean(payload && (payload.error || payload.isError));

  if (category === 'error') {
    return hasError;
  }
  if (category === 'tool') {
    return type === 'tool_call' || type === 'tool_result';
  }
  if (category === 'model') {
    return type === 'model_response' || type === 'reasoning' || type === 'step_end';
  }
  if (category === 'system') {
    return (
      type === 'user_msg' ||
      type === 'system_prompt' ||
      type === 'context_injection' ||
      type === 'turn_start' ||
      type === 'step_start' ||
      type === 'compaction' ||
      type === 'turn_end'
    );
  }
  return true;
}

export function Trajectory({
  conversationId,
  onFork,
}: {
  conversationId: string | null;
  onFork?: (newConversationId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [category, setCategory] = useState<EventCategory>('all');
  const [filter, setFilter] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [forkingSeq, setForkingSeq] = useState<number | null>(null);

  async function load() {
    if (!nativeAvailable || !conversationId || loading) return;
    setLoading(true);
    setError('');
    try {
      const items = await api.getSessionEvents(conversationId);
      setEvents(items);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (open && conversationId) {
      void load();
    } else if (!conversationId) {
      setEvents([]);
    }
  }, [conversationId, open]);

  async function handleFork(seq: number) {
    if (!conversationId || forkingSeq !== null) return;
    setForkingSeq(seq);
    setError('');
    try {
      const result = await api.forkSession(conversationId, seq);
      if (onFork) {
        onFork(result.newConversation.id);
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setForkingSeq(null);
    }
  }

  const query = filter.trim().toLowerCase();
  const visible = events.filter(event => {
    if (!matchesCategory(event, category)) return false;
    if (!query) return true;
    const type = event.eventType.toLowerCase();
    const toolCallId = (event.toolCallId || '').toLowerCase();
    const stepId = (event.stepId || '').toLowerCase();
    const payloadStr = JSON.stringify(event.payload).toLowerCase();
    return (
      type.includes(query) ||
      toolCallId.includes(query) ||
      stepId.includes(query) ||
      payloadStr.includes(query)
    );
  });

  return (
    <details
      className="trajectory"
      open={open}
      onToggle={e => {
        const isOpen = (e.target as HTMLDetailsElement).open;
        setOpen(isOpen);
        if (isOpen) void load();
      }}
    >
      <summary className="trajectory-summary">
        <span>Trajectory{conversationId ? ` · ${events.length} events` : ''}</span>
      </summary>

      <div className="trajectory-body">
        <div className="trajectory-toolbar">
          <div className="trajectory-categories" role="tablist" aria-label="Event category filters">
            {(['all', 'model', 'tool', 'system', 'error'] as const).map(cat => (
              <button
                key={cat}
                type="button"
                className={`trajectory-tab ${category === cat ? 'active' : ''}`}
                onClick={() => setCategory(cat)}
              >
                {cat.charAt(0).toUpperCase() + cat.slice(1)}
              </button>
            ))}
          </div>

          <div className="trajectory-search-box">
            <input
              aria-label="Filter trajectory events"
              type="search"
              placeholder="Search trajectory events…"
              value={filter}
              onChange={e => setFilter(e.target.value)}
            />
            <button
              type="button"
              className="trajectory-refresh-btn"
              disabled={loading || !conversationId}
              onClick={() => void load()}
              title="Refresh trajectory"
            >
              <RefreshCw size={13} className={loading ? 'spinning' : ''} />
              <span>{loading ? 'Loading…' : 'Refresh'}</span>
            </button>
          </div>
        </div>

        {error && (
          <div className="trajectory-error" role="alert">
            <AlertTriangle size={14} />
            <span>{error}</span>
          </div>
        )}

        {!conversationId && (
          <p className="trajectory-empty">Start a conversation to see its run trajectory.</p>
        )}
        {conversationId && !loading && !events.length && !error && (
          <p className="trajectory-empty">No events recorded for this conversation yet.</p>
        )}
        {conversationId && !loading && events.length > 0 && !visible.length && (
          <p className="trajectory-empty">No events match this filter.</p>
        )}

        <div className="trajectory-events-list">
          {visible.map(event => (
            <details
              key={event.id || `${event.conversationId}-${event.seq}`}
              className="trajectory-event"
            >
              <summary className="trajectory-event-header">
                <div className="trajectory-event-meta">
                  <span className="trajectory-event-seq">#{event.seq}</span>
                  <span className={`trajectory-event-badge badge-${event.eventType}`}>
                    {event.eventType}
                  </span>
                  {event.stepId && <span className="trajectory-event-step">{event.stepId}</span>}
                  {event.toolCallId && (
                    <span className="trajectory-event-tool">{event.toolCallId}</span>
                  )}
                </div>
                <div className="trajectory-event-actions">
                  <button
                    type="button"
                    className="trajectory-fork-btn"
                    disabled={forkingSeq !== null}
                    onClick={e => {
                      e.preventDefault();
                      e.stopPropagation();
                      void handleFork(event.seq);
                    }}
                    title={`Fork session up to event #${event.seq}`}
                  >
                    <GitFork size={12} />
                    <span>{forkingSeq === event.seq ? 'Forking…' : 'Fork from here'}</span>
                  </button>
                </div>
              </summary>
              <div className="trajectory-event-details">
                <div className="trajectory-event-info">
                  <small>ID: {event.id}</small>
                  {event.createdAt > 0 && (
                    <small>Time: {new Date(event.createdAt).toLocaleTimeString()}</small>
                  )}
                  {event.ignorable && <small className="trajectory-ignorable">(ignorable)</small>}
                </div>
                <pre className="trajectory-event-payload">{previewPayload(event.payload)}</pre>
              </div>
            </details>
          ))}
        </div>
      </div>
    </details>
  );
}
