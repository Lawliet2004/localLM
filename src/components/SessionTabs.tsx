import { useRef } from 'react';
import { Circle, LoaderCircle, MessageSquare, Plus, X } from 'lucide-react';
import type { Conversation } from '../lib/types';
import type { SessionRun } from '../hooks/useSessionRuns';

export function readOpenSessions(): string[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem('locallm-open-sessions') ?? '[]');
    return Array.isArray(value) ? [...new Set(value.filter((id): id is string => typeof id === 'string' && id.length > 0 && id.length <= 128))].slice(-64) : [];
  } catch { return []; }
}

export function sessionStatus(run?: SessionRun, recoverable = false): string {
  if (run?.approvals.length) return 'Needs your input';
  if (run?.generating && run.activity?.state === 'queued') return 'Waiting for a slot';
  if (run?.generating) return 'Working';
  if (recoverable) return 'Unsent request';
  if (run?.error) return 'Needs attention';
  if (run?.unread) return 'New response';
  return '';
}

export function SessionTabs({ ids, conversations, activeId, runs, pendingIds = [], disabled, onSelect, onClose, onNew }: {
  ids: string[]; conversations: Conversation[]; activeId: string | null;
  runs: Record<string, SessionRun>; disabled: boolean;
  pendingIds?: readonly string[];
  onSelect: (id: string) => void; onClose: (id: string) => void; onNew: () => void;
}) {
  const strip = useRef<HTMLDivElement>(null);
  const byId = new Map(conversations.map(item => [item.id, item]));
  const visible = ids.flatMap(id => byId.has(id) ? [byId.get(id)!] : []);
  if (!visible.length) return null;
  return <nav className="session-strip" aria-label="Open conversations">
    <div className="session-tabs" role="tablist" aria-label="Conversation tabs" ref={strip} onKeyDown={event => {
      if (disabled || (event.target as HTMLElement).getAttribute('role') !== 'tab') return;
      const index = visible.findIndex(item => item.id === (event.target as HTMLElement).dataset.sessionId);
      const next = event.key === 'ArrowRight' ? (index + 1) % visible.length
        : event.key === 'ArrowLeft' ? (index - 1 + visible.length) % visible.length
        : event.key === 'Home' ? 0 : event.key === 'End' ? visible.length - 1 : -1;
      if (next < 0) return;
      event.preventDefault();
      strip.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
      onSelect(visible[next].id);
    }}>
      {visible.map((item, index) => {
        const run = runs[item.id], status = sessionStatus(run, pendingIds.includes(item.id)), selected = activeId === item.id;
        return <div key={item.id} className={`session-tab-item ${selected ? 'selected' : ''}`}>
          <button type="button" role="tab" aria-selected={selected} aria-controls="active-session-content" data-session-id={item.id}
            aria-label={`${item.title}${status ? ` · ${status}` : ''}`} title={`${item.title}${status ? ` — ${status}` : ''}`}
            tabIndex={selected || !activeId && index === 0 ? 0 : -1} disabled={disabled} onClick={() => onSelect(item.id)}>
            {run?.generating ? <LoaderCircle size={13} className="session-working" aria-hidden="true" /> : <MessageSquare size={13} aria-hidden="true" />}
            <span className="session-tab-title">{item.title}</span>
            {status && <span className={`session-status ${run?.approvals.length || run?.error ? 'attention' : ''}`}><Circle size={7} fill="currentColor" aria-hidden="true" /><span className="sr-only">{status}</span></span>}
          </button>
          <button type="button" className="session-tab-close" aria-label={`Close tab ${item.title}`} title="Close tab; work keeps running" disabled={disabled} onClick={() => onClose(item.id)}><X size={12} /></button>
        </div>;
      })}
    </div>
    <button className="icon-button session-new" type="button" aria-label="Open new conversation" title="New conversation (Ctrl+N)" disabled={disabled} onClick={onNew}><Plus size={15} /></button>
  </nav>;
}
