import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Schedule } from '../lib/types';

const empty: Schedule = {
  id: '', name: '', cron: '0 9 * * 1-5', task: '', conversationId: null,
  allowWrite: false, enabled: true, runOnce: false, lastRunAt: null, lastResult: null, createdAt: 0,
};

export function Schedules() {
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [draft, setDraft] = useState(empty);
  const [webhook, setWebhook] = useState({ enabled: false, port: 4317, hasToken: false });
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(nativeAvailable);
  async function refresh() {
    if (!nativeAvailable) return;
    try {
      setSchedules(await api.listSchedules());
      setWebhook(await api.webhookState());
    } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  async function save() {
    setError(''); setNotice('');
    try {
      await api.saveSchedule({ ...draft, id: '', createdAt: Date.now() });
      setDraft(empty); setNotice('Schedule saved. It runs inside the app through the same agent states and approval policy.');
      await refresh();
    } catch (e) { setError(errorMessage(e)); }
  }
  async function remove(id: string) {
    setError('');
    try { await api.deleteSchedule(id); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function runNow(id: string) {
    setError(''); setNotice('');
    try { setNotice(await api.runScheduleNow(id)); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function rotate() {
    setError('');
    try { setToken(await api.rotateWebhookToken()); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
  }
  return (
    <section aria-label="Schedules">
      <h2>Schedules</h2>
      <p>Cron-like tasks (minute hour day month weekday). Unattended writes need allow-write AND conversation Full access; otherwise runs pin to trusted reads.</p>
      {error && <div className="error-banner" role="alert"><span>{error}</span></div>}
      {notice && <div className="preview-banner" role="status">{notice}</div>}
      <label>Name <input aria-label="Schedule name" value={draft.name} maxLength={120} onChange={e => setDraft({ ...draft, name: e.target.value })} /></label>
      <label>Cron <input aria-label="Schedule cron" value={draft.cron} onChange={e => setDraft({ ...draft, cron: e.target.value })} placeholder="0 9 * * 1-5" /></label>
      <label>Task <input aria-label="Schedule task" value={draft.task} maxLength={4000} onChange={e => setDraft({ ...draft, task: e.target.value })} placeholder="Run the test suite and summarize" /></label>
      <label><input type="checkbox" checked={draft.allowWrite} onChange={e => setDraft({ ...draft, allowWrite: e.target.checked })} /> Allow unattended writes</label>
      <button type="button" disabled={busy || !draft.name.trim() || !draft.task.trim()} onClick={() => void save()}>Save schedule</button>
      <ul className="schedule-list">{schedules.map(schedule => (
        <li key={schedule.id}><span><strong>{schedule.name}</strong> <code>{schedule.cron}</code></span>
          <small>{schedule.enabled ? 'Enabled' : 'Disabled'} · last: {schedule.lastRunAt ? new Date(schedule.lastRunAt).toLocaleString() : 'never'}</small>
          {schedule.lastResult && <p>{schedule.lastResult}</p>}
          <button type="button" className="secondary" onClick={() => void runNow(schedule.id)}>Run now</button>
          <button type="button" className="icon-button" aria-label={`Delete ${schedule.name}`} onClick={() => void remove(schedule.id)}>Delete</button></li>
      ))}</ul>
      <h2>Webhook ingress</h2>
      <p>Loopback only ({webhook.port}), bearer-authed, 64 KiB bodies. Listener restarts apply on next launch. Headless CLI: <code>locallm --profile headless "task" --wait</code>.</p>
      <label><input type="checkbox" checked={webhook.enabled} onChange={async e => { try { setWebhook(await api.setWebhook(e.target.checked)); } catch (err) { setError(errorMessage(err)); } }} /> Enable listener</label>
      <button type="button" className="secondary" onClick={() => void rotate()}>Rotate token</button>
      {token && <p role="status">New token (shown once): <code>{token}</code></p>}
      {!webhook.hasToken && <p>Token: none yet — rotate one to serve requests.</p>}
    </section>
  );
}
