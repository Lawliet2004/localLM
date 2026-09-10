import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Fact } from '../lib/types';

export function Memory() {
  const [scope, setScope] = useState('global');
  const [facts, setFacts] = useState<Fact[]>([]);
  const [draft, setDraft] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!nativeAvailable) return;
    api.getWorkspace().then(value => { if (value.path) setScope(value.path); }).catch(() => {});
  }, []);
  async function refresh(target: string) {
    if (!nativeAvailable) return;
    setBusy(true); setError('');
    try { setFacts(await api.listFacts(target, 50)); }
    catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { void refresh(scope); }, [scope]);
  async function teach() {
    if (!draft.trim()) return;
    setError(''); setNotice('');
    try {
      await api.teachFact(scope, draft.trim());
      setDraft(''); setNotice('Taught. New sessions recall it verbatim.');
      await refresh(scope);
    } catch (e) { setError(errorMessage(e)); }
  }
  async function forget(id: string) {
    setError('');
    try { await api.forgetFact(id); await refresh(scope); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function ingest() {
    setError(''); setNotice('');
    try {
      await api.ingestRepo(scope);
      setNotice('Workspace surveyed into memory.');
      await refresh(scope);
    } catch (e) { setError(errorMessage(e)); }
  }
  return (
    <div className="settings-page">
      <div className="page-heading"><p className="eyebrow">LOCAL MEMORY</p><h1>Memory</h1>
        <p>Facts stay in this device's database, scoped to a workspace. The harness recalls them at task start; providers only see what the prompt carries.</p></div>
      {error && <div className="error-banner" role="alert"><span>{error}</span></div>}
      {notice && <div className="preview-banner" role="status">{notice}</div>}
      <label>Scope <input aria-label="Memory scope" value={scope} onChange={e => setScope(e.target.value)} placeholder="workspace path or global" /></label>
      <div className="row-actions">
        <button type="button" className="secondary" disabled={busy} onClick={() => void refresh(scope)}>Refresh</button>
        <button type="button" className="secondary" disabled={busy || scope === 'global'} onClick={() => void ingest()}>Survey workspace</button>
      </div>
      <label>Teach once <input aria-label="Teach a fact" value={draft} maxLength={2000} onChange={e => setDraft(e.target.value)} placeholder="use pnpm not npm" /></label>
      <button type="button" disabled={busy || !draft.trim()} onClick={() => void teach()}>Teach</button>
      <ul className="fact-list">{facts.map(fact => (
        <li key={fact.id}><span>{fact.fact}</span><small>{fact.origin} · {fact.scope}</small>
          <button type="button" className="icon-button" aria-label={`Forget ${fact.fact.slice(0, 40)}`} onClick={() => void forget(fact.id)}>Forget</button></li>
      ))}</ul>
      {!facts.length && !busy && <p>No facts in this scope yet.</p>}
    </div>
  );
}
