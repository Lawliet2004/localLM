import { useCallback, useEffect, useState } from 'react';
import { Ban, Brain, FileText, Map, Pin, Plus, Trash2 } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ContextPreview, MemoryFact } from '../lib/types';

const KIND_LABEL: Record<string, string> = {
  rules: 'rules',
  'repo-map': 'repo map',
  memory: 'memory',
  pinned: 'pinned memory',
};

function scopeLabel(scope: string): string {
  if (scope === 'user') return 'user';
  if (scope.startsWith('conversation:')) return 'this chat';
  return 'project';
}

function FactRow({ fact, conversation, onChanged }: { fact: MemoryFact; conversation: string; onChanged: () => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(fact.fact);
  const [error, setError] = useState('');
  const run = async (action: () => Promise<unknown>) => {
    setError('');
    try {
      await action();
      onChanged();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };
  return (
    <li className={`context-fact ${fact.excluded ? 'is-excluded' : ''}`}>
      {editing
        ? <div className="context-fact-edit">
            <input value={draft} onChange={event => setDraft(event.target.value)} maxLength={2000} aria-label="Edit memory text"
              onKeyDown={event => { if (event.key === 'Enter') void run(() => api.memoryUpdate(conversation, fact.id, draft)); }} />
            <button type="button" onClick={() => void run(() => api.memoryUpdate(conversation, fact.id, draft))}>Save</button>
            <button type="button" className="secondary" onClick={() => { setEditing(false); setDraft(fact.fact); }}>Cancel</button>
          </div>
        : <p className="context-fact-text">{fact.fact}</p>}
      <div className="context-fact-meta">
        <span className="context-tag">{scopeLabel(fact.scope)}</span>
        <span className="context-tag context-tag-muted">{fact.origin}</span>
        {fact.pinned && <span className="context-tag">pinned</span>}
        {fact.excluded && <span className="context-tag context-tag-excluded">excluded</span>}
        {fact.stale && <span className="context-tag context-tag-stale" title="The source file or branch changed since this fact was captured.">stale</span>}
        {fact.reason && <span className="context-fact-reason" title={fact.reason}>{fact.reason}</span>}
        {fact.sourcePath && <span className="context-fact-source" title={fact.sourcePath}>{fact.sourcePath.split(/[\\/]/).pop()}</span>}
        <span className="context-fact-actions">
          <button type="button" className="icon-button" aria-label={fact.pinned ? 'Unpin memory' : 'Pin memory'} title={fact.pinned ? 'Unpin' : 'Pin (recalled first)'}
            onClick={() => void run(() => api.memorySetPinned(conversation, fact.id, !fact.pinned))}><Pin size={13} /></button>
          <button type="button" className="icon-button" aria-label={fact.excluded ? 'Include memory' : 'Exclude memory'} title={fact.excluded ? 'Include again' : 'Exclude from context (kept, never injected)'}
            onClick={() => void run(() => api.memorySetExcluded(conversation, fact.id, !fact.excluded))}><Ban size={13} /></button>
          <button type="button" className="icon-button" aria-label="Edit memory" title="Edit" onClick={() => setEditing(true)}><FileText size={13} /></button>
          <button type="button" className="icon-button" aria-label="Delete memory" title="Delete" onClick={() => void run(() => api.memoryDelete(conversation, fact.id))}><Trash2 size={13} /></button>
        </span>
      </div>
      {error && <p className="error-banner" role="alert">{error}</p>}
    </li>
  );
}

/// Context tab: what the next turn will inject (rules, repo map, memories)
/// plus the memory list for this conversation's scopes.
export function ContextPanel({ conversation }: { conversation?: string | null }) {
  const [preview, setPreview] = useState<ContextPreview | null>(null);
  const [facts, setFacts] = useState<MemoryFact[]>([]);
  const [autoRetain, setAutoRetain] = useState<boolean | null>(null);
  const [draft, setDraft] = useState('');
  const [scope, setScope] = useState<'project' | 'conversation' | 'user'>('project');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(() => {
    if (!nativeAvailable || !conversation || conversation === 'new') {
      setLoading(false);
      return;
    }
    Promise.all([
      api.contextPreview(conversation).then(setPreview),
      api.memoryFacts(conversation).then(setFacts),
      api.memoryAutoRetain().then(setAutoRetain),
    ]).catch(caught => setError(errorMessage(caught))).finally(() => setLoading(false));
  }, [conversation]);

  useEffect(refresh, [refresh]);

  const add = async () => {
    if (!conversation) return;
    setError('');
    try {
      await api.memoryCreate(conversation, scope, draft);
      setDraft('');
      refresh();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };

  if (!conversation || conversation === 'new') {
    return <p className="panel-empty">Open a conversation to inspect its context.</p>;
  }
  const used = preview?.totalTokens ?? 0;
  const budget = preview?.budgetTokens ?? 0;
  return (
    <div className="context-view">
      {loading && <p role="status">Loading context…</p>}
      {error && <p className="error-banner" role="alert">{error}</p>}
      {preview && (
        <section className="context-section" aria-label="Context preview">
          <p className="context-section-label">Included next turn · ~{used.toLocaleString()} tokens</p>
          <div className="context-budget" role="meter" aria-label="Context budget" aria-valuenow={used} aria-valuemax={budget || undefined}>
            <div className="context-budget-fill" style={{ width: budget ? `${Math.min(100, (used / budget) * 100)}%` : 0 }} />
          </div>
          {preview.items.length === 0 && <p className="panel-empty">No rules or memories will be injected.</p>}
          <ul className="context-items">
            {preview.items.map((item, index) => (
              <li key={item.id ?? `${item.kind}-${index}`} className="context-item">
                <span className="context-tag">{KIND_LABEL[item.kind] ?? item.kind}</span>
                <span className="context-item-label" title={item.source ?? undefined}>{item.label}</span>
                <span className="context-item-reason">{item.reason}</span>
                <span className="context-item-tokens">~{item.tokens} tok</span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <section className="context-section" aria-label="Memories">
        <p className="context-section-label"><Brain size={12} aria-hidden="true" /> Memories</p>
        <label className="context-retain">
          <input type="checkbox" checked={autoRetain ?? true} disabled={autoRetain === null}
            onChange={event => {
              const enabled = event.target.checked;
              setAutoRetain(enabled);
              void api.memorySetAutoRetain(enabled).catch(caught => { setAutoRetain(!enabled); setError(errorMessage(caught)); });
            }} />
          Let the agent store memories automatically
        </label>
        {facts.length === 0 && <p className="panel-empty">Nothing remembered for this conversation's scopes.</p>}
        <ul className="context-facts">
          {facts.map(fact => <FactRow key={fact.id} fact={fact} conversation={conversation} onChanged={refresh} />)}
        </ul>
        <div className="context-add">
          <select value={scope} onChange={event => setScope(event.target.value as typeof scope)} aria-label="Memory scope">
            <option value="project">Project</option>
            <option value="conversation">This chat</option>
            <option value="user">User</option>
          </select>
          <input value={draft} onChange={event => setDraft(event.target.value)} maxLength={2000}
            placeholder="Remember a fact…" aria-label="New memory"
            onKeyDown={event => { if (event.key === 'Enter' && draft.trim()) void add(); }} />
          <button type="button" onClick={() => void add()} disabled={!draft.trim()}><Plus size={13} aria-hidden="true" /> Add</button>
        </div>
      </section>
      <p className="evidence-note"><Map size={12} aria-hidden="true" /> Injected text is context only — it cannot change permissions, modes, or tools.</p>
    </div>
  );
}
