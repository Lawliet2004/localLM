import { useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Message } from '../lib/types';

interface Hit { conversationId: string; conversationTitle: string; messageId: string; role: string; excerpt: string; createdAt: number }

export function SessionsPanel({ conversationId, messages, onForked }: {
  conversationId: string | null; messages: Message[]; onForked: (id: string) => void;
}) {
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<Hit[]>([]);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);

  async function search() {
    if (!nativeAvailable || !query.trim() || busy) return;
    setBusy(true); setError('');
    try { setHits(await api.searchSessions(query.trim(), 30)); }
    catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }

  async function fork(throughMessageId: string) {
    if (!nativeAvailable || !conversationId || busy) return;
    setBusy(true); setError(''); setNotice('');
    try {
      const forked = await api.forkSession(conversationId, throughMessageId);
      setNotice(`Forked as "${forked.title}".`);
      onForked(forked.id);
    } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }

  async function replay() {
    if (!nativeAvailable || !conversationId || busy) return;
    setBusy(true); setError('');
    try {
      const transcript = await api.replaySession(conversationId);
      const count = (transcript.history as unknown[]).length;
      setNotice(`Transcript re-derived: ${count} model-visible entries (deterministic over stored rows; a live re-run is not).`);
    } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }

  return (
    <details className="sessions-panel">
      <summary>Sessions · fork · replay · search</summary>
      <div className="sessions-body">
        {error && <p role="alert">{error}</p>}
        {notice && <p role="status">{notice}</p>}
        <div className="row-actions">
          <input aria-label="Search past sessions" type="search" placeholder="Search tool calls and messages…" value={query} onChange={e => setQuery(e.target.value)} />
          <button type="button" className="secondary" disabled={busy || !query.trim()} onClick={() => void search()}>Search</button>
          <button type="button" className="secondary" disabled={busy || !conversationId} onClick={() => void replay()}>Replay transcript</button>
        </div>
        {hits.length > 0 && <ul>{hits.map(hit => <li key={hit.messageId}><strong>{hit.conversationTitle}</strong> · {hit.role} · {hit.excerpt}</li>)}</ul>}
        {conversationId && messages.length > 0 && (
          <ul className="fork-list">{messages.map(message => (
            <li key={message.id}><small>{message.role} · {message.content.slice(0, 80)}</small>{' '}
              <button type="button" className="secondary" disabled={busy} onClick={() => void fork(message.id)}>Fork from here</button></li>
          ))}</ul>
        )}
      </div>
    </details>
  );
}
