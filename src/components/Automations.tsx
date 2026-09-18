import { useEffect, useState } from 'react';
import { Clock3, Plus, Play, Pause, Trash2, Pencil } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Schedule } from '../lib/types';

const blank: Schedule = { id: '', name: '', cron: '0 9 * * *', task: '', conversationId: null, allowWrite: false, enabled: true, runOnce: false, lastRunAt: null, lastResult: null, createdAt: 0 };
export function Automations({ conversationId }: { conversationId: string | null }) {
  const [items, setItems] = useState<Schedule[]>([]);
  const [draft, setDraft] = useState<Schedule | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  async function refresh() { setItems(await api.listSchedules()); }
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError('');
    try { await action(); await refresh(); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { if (nativeAvailable) void act(refresh); }, []);
  return <section className="management-page">
    <div className="page-heading"><div><Clock3 size={24} /><h1>Automations</h1><p>Schedule work in your local workspace.</p></div><button className="primary" disabled={busy || !nativeAvailable} onClick={() => setDraft({ ...blank, conversationId })}><Plus size={16} /> New automation</button></div>
    <p className="muted">Keep LocalLM running for scheduled work. Cron times use UTC. Unattended writes require explicit write permission and Full access on the task.</p>
    {error && <div role="alert" className="error-banner">{error}<button onClick={() => void act(refresh)}>Retry</button></div>}
    {notice && <p role="status">{notice}</p>}
    {draft && <form className="management-form" onSubmit={e => { e.preventDefault(); void act(async () => { await api.saveSchedule(draft); setDraft(null); }); }}>
      <label>Name<input required maxLength={120} value={draft.name} onChange={e => setDraft({ ...draft, name: e.target.value })} /></label>
      <label>Instructions<textarea required maxLength={4000} rows={4} value={draft.task} onChange={e => setDraft({ ...draft, task: e.target.value })} /></label>
      <label>Schedule (UTC cron)<input required aria-describedby="cron-help" value={draft.cron} onChange={e => setDraft({ ...draft, cron: e.target.value })} /></label>
      <small id="cron-help">minute hour day month weekday · 0 9 * * * = daily at 09:00 UTC</small>
      <label><input type="checkbox" checked={draft.runOnce} onChange={e => setDraft({ ...draft, runOnce: e.target.checked })} /> Run once</label>
      <label><input type="checkbox" checked={draft.allowWrite} onChange={e => setDraft({ ...draft, allowWrite: e.target.checked })} /> Allow writes under this task’s access policy</label>
      <div className="button-row"><button className="primary" disabled={busy}>Save automation</button><button type="button" disabled={busy} onClick={() => setDraft(null)}>Cancel</button></div>
    </form>}
    {!items.length && !draft && <div className="empty-panel">No automations yet. Schedule a recurring check or a one-time task.</div>}
    {items.map(item => <article className="management-item" key={item.id}>
      <div><strong>{item.name}</strong><p>{item.task}</p><small>{item.cron} UTC · {item.enabled ? 'Active' : 'Paused'}{item.lastRunAt ? ` · Last run ${new Date(item.lastRunAt * 1000).toLocaleString()}` : ' · Has not run yet'}</small></div>
      <div className="button-row">
        <button disabled={busy} aria-label={`Run ${item.name}`} onClick={() => void act(async () => setNotice(await api.runScheduleNow(item.id)))}><Play size={15} /></button>
        <button disabled={busy} aria-label={`${item.enabled ? 'Pause' : 'Enable'} ${item.name}`} onClick={() => void act(() => api.saveSchedule({ ...item, enabled: !item.enabled }))}>{item.enabled ? <Pause size={15} /> : <Play size={15} />}</button>
        <button disabled={busy} aria-label={`Edit ${item.name}`} onClick={() => setDraft(item)}><Pencil size={15} /></button>
        <button disabled={busy} aria-label={`Delete ${item.name}`} onClick={() => void act(() => api.deleteSchedule(item.id))}><Trash2 size={15} /></button>
      </div>
      {item.lastResult && <details><summary>Last result</summary><pre>{item.lastResult}</pre></details>}
    </article>)}
  </section>;
}
