import { useEffect, useState } from 'react';
import { api, errorMessage } from '../lib/api';
import type { Hook } from '../lib/types';

const EVENTS: { value: Hook['event']; label: string }[] = [
  { value: 'pre_tool_use', label: 'Before tool' },
  { value: 'post_tool_use', label: 'After tool' },
  { value: 'run_finished', label: 'Run finished' },
];

/** User shell hooks around tool dispatch. These run commands in the task
 * workspace — they are not model tools and are invisible to the model. */
export function HooksSettings() {
  const [hooks, setHooks] = useState<Hook[] | null>(null);
  const [error, setError] = useState('');
  const [status, setStatus] = useState('');

  useEffect(() => {
    api.getHooks().then(setHooks).catch(e => setError(errorMessage(e)));
  }, []);

  const update = (index: number, patch: Partial<Hook>) =>
    setHooks(current => current?.map((hook, i) => (i === index ? { ...hook, ...patch } : hook)) ?? null);
  const remove = (index: number) =>
    setHooks(current => current?.filter((_, i) => i !== index) ?? null);
  const add = () =>
    setHooks(current => [
      ...(current ?? []),
      { id: `hook-${Date.now()}`, event: 'pre_tool_use', matcher: '*', command: '', timeoutSecs: 30, enabled: true },
    ]);
  const save = async () => {
    if (!hooks) return;
    setStatus('Saving…');
    try {
      await api.saveHooks(hooks);
      setStatus('Saved.');
      setError('');
    } catch (e) {
      setStatus('');
      setError(errorMessage(e));
    }
  };

  return <section className="settings-section" aria-labelledby="hooks-title">
    <h2 id="hooks-title">Hooks</h2>
    <p className="section-description">
      Shell commands that run around tool calls and run completion in the task workspace. A
      <em> Before tool </em> hook exiting with code 2 blocks the call; its stderr reaches the model.
      An <em> After tool </em> hook's stdout is appended to the tool result as a note.
    </p>
    {hooks === null && !error && <p role="status">Loading…</p>}
    {hooks?.map((hook, index) => <div className="hook-row" key={hook.id || index}>
      <input aria-label={`Hook ${index + 1} enabled`} type="checkbox" checked={hook.enabled} onChange={e => update(index, { enabled: e.target.checked })} />
      <select aria-label={`Hook ${index + 1} event`} value={hook.event} onChange={e => update(index, { event: e.target.value as Hook['event'] })}>
        {EVENTS.map(event => <option key={event.value} value={event.value}>{event.label}</option>)}
      </select>
      <input aria-label={`Hook ${index + 1} matcher`} spellCheck={false} value={hook.matcher} placeholder="workspace_*" onChange={e => update(index, { matcher: e.target.value })} />
      <input aria-label={`Hook ${index + 1} command`} spellCheck={false} value={hook.command} placeholder="shell command" onChange={e => update(index, { command: e.target.value })} />
      <input aria-label={`Hook ${index + 1} timeout`} inputMode="numeric" value={hook.timeoutSecs} onChange={e => update(index, { timeoutSecs: Number(e.target.value) || 30 })} />
      <button type="button" className="secondary" aria-label={`Remove hook ${index + 1}`} onClick={() => remove(index)}>Remove</button>
    </div>)}
    <div className="connector-actions">
      <button type="button" className="secondary" onClick={add}>Add hook</button>
      <button type="button" className="primary" disabled={!hooks} onClick={() => void save()}>Save hooks</button>
      {status && <p role="status">{status}</p>}
    </div>
    {error && <p role="alert">{error}</p>}
  </section>;
}
