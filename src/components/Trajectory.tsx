import { useState, useEffect, useRef } from 'react';
import { GitFork, RefreshCw, AlertTriangle } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { SessionEvent } from '../lib/types';

export type EventCategory = 'all' | 'model' | 'tool' | 'system' | 'error';

const EVENT_PAGE_SIZE = 500;

function mergePage(current: SessionEvent[], batch: SessionEvent[]): SessionEvent[] {
  return [...new Map([...current, ...batch].map(e => [e.seq, e])).values()].sort((a, b) => a.seq - b.seq);
}

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
  const [loadingMore, setLoadingMore] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [forkingSeq, setForkingSeq] = useState<number | null>(null);
  // Requests are tagged with a generation and the conversation that owns them so a
  // late response can never write rows, errors, or loading state for another view.
  const requestGeneration = useRef(0);
  const eventsOwner = useRef<string | null>(null);

  async function load() {
    if (!nativeAvailable || !conversationId) return;
    const generation = ++requestGeneration.current;
    eventsOwner.current = conversationId;
    setLoading(true);
    setLoadingMore(false);
    setError('');
    setEvents([]);
    setHasMore(false);
    setForkingSeq(null);
    try {
      const firstPage = await api.getSessionEvents(conversationId, null, EVENT_PAGE_SIZE);
      if (requestGeneration.current !== generation || eventsOwner.current !== conversationId) return;
      setEvents(firstPage);
      setHasMore(firstPage.length >= EVENT_PAGE_SIZE);
    } catch (e) {
      if (requestGeneration.current === generation) setError(errorMessage(e));
    } finally {
      if (requestGeneration.current === generation) setLoading(false);
    }
  }

  async function loadMore() {
    if (!nativeAvailable || !conversationId || !hasMore || loading || loadingMore) return;
    if (eventsOwner.current !== conversationId) return;
    const generation = requestGeneration.current;
    const afterSeq = events.reduce((max, e) => Math.max(max, e.seq), 0) + 1;
    setLoadingMore(true);
    try {
      const batch = await api.getSessionEvents(conversationId, afterSeq, EVENT_PAGE_SIZE);
      if (requestGeneration.current !== generation || eventsOwner.current !== conversationId) return;
      setEvents(current => mergePage(current, batch));
      if (batch.length < EVENT_PAGE_SIZE) setHasMore(false);
    } catch (e) {
      if (requestGeneration.current === generation) setError(errorMessage(e));
    } finally {
      if (requestGeneration.current === generation) setLoadingMore(false);
    }
  }

  useEffect(() => {
    if (open && conversationId) {
      void load();
    } else if (!conversationId || eventsOwner.current !== conversationId) {
      // The selected conversation changed while closed or was removed: drop the
      // previous conversation's rows and any in-flight request for it.
      requestGeneration.current += 1;
      eventsOwner.current = conversationId;
      setEvents([]);
      setHasMore(false);
      setError('');
      setLoading(false);
      setLoadingMore(false);
      setForkingSeq(null);
    }
    return () => {
      // Invalidate any request started by this effect run on change/unmount.
      requestGeneration.current += 1;
    };
  }, [conversationId, open]);

  async function handleFork(event: SessionEvent) {
    // Fork the conversation that produced this row, not whatever happens to be
    // selected now — displayed sequence numbers belong to the event's owner.
    if (forkingSeq !== null) return;
    const generation = requestGeneration.current;
    setForkingSeq(event.seq);
    setError('');
    try {
      const result = await api.forkSession(event.conversationId, event.seq);
      if (onFork && requestGeneration.current === generation && eventsOwner.current === event.conversationId) {
        onFork(result.newConversation.id);
      }
    } catch (e) {
      if (eventsOwner.current === event.conversationId) setError(errorMessage(e));
    } finally {
      if (eventsOwner.current === event.conversationId) setForkingSeq(null);
    }
  }

  const ownedEvents = conversationId ? events.filter(event => event.conversationId === conversationId) : [];
  const ownerCurrent = conversationId !== null && eventsOwner.current === conversationId;
  const query = filter.trim().toLowerCase();
  const visible = ownedEvents.filter(event => {
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

  const shownError = ownerCurrent ? error : '';
  const busy = ownerCurrent && loading;
  const busyMore = ownerCurrent && loadingMore;
  const moreAvailable = ownerCurrent && hasMore;

  return (
    <details
      className="trajectory"
      open={open}
      onToggle={e => {
        setOpen((e.target as HTMLDetailsElement).open);
      }}
    >
      <summary className="trajectory-summary">
        {/* Events load lazily on first open; without `ownerCurrent` a fresh
            conversation would forever advertise a misleading "0 events". */}
        <span>Trajectory{conversationId && ownerCurrent ? ` · ${ownedEvents.length}${moreAvailable ? '+' : ''} events` : ''}</span>
      </summary>

      <div className="trajectory-body">
        <div className="trajectory-toolbar">
          <div className="trajectory-categories" role="group" aria-label="Event category filters">
            {(['all', 'model', 'tool', 'system', 'error'] as const).map(cat => (
              <button
                key={cat}
                type="button"
                aria-pressed={category === cat}
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
              disabled={busy || !conversationId}
              onClick={() => void load()}
              title="Refresh trajectory"
            >
              <RefreshCw size={13} className={busy ? 'spinning' : ''} />
              <span>{busy ? 'Loading…' : 'Refresh'}</span>
            </button>
          </div>
        </div>

        {shownError && (
          <div className="trajectory-error" role="alert">
            <AlertTriangle size={14} />
            <span>{shownError}</span>
          </div>
        )}

        {!conversationId && (
          <p className="trajectory-empty">Start a conversation to see its run trajectory.</p>
        )}
        {conversationId && !busy && !ownedEvents.length && !shownError && (
          <p className="trajectory-empty">No events recorded for this conversation yet.</p>
        )}
        {conversationId && !busy && ownedEvents.length > 0 && !visible.length && (
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
                      void handleFork(event);
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

        {conversationId && moreAvailable && (
          <button
            type="button"
            className="trajectory-refresh-btn"
            style={{ alignSelf: 'flex-start' }}
            disabled={busyMore}
            onClick={() => void loadMore()}
          >
            <RefreshCw size={13} className={busyMore ? 'spinning' : ''} />
            <span>{busyMore ? 'Loading…' : 'Load more events'}</span>
          </button>
        )}
        {conversationId && !moreAvailable && !busy && ownedEvents.length > EVENT_PAGE_SIZE && (
          <p className="trajectory-empty">All {ownedEvents.length} events loaded.</p>
        )}
      </div>
    </details>
  );
}
