import { useEffect, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { PluginView } from '../lib/types';

const BLANK = (): PluginView => ({
  name: '',
  description: '',
  command: [],
  arguments: {},
  required: [],
  timeoutSeconds: 60,
  path: '',
  status: 'ok',
});

/**
 * User-authored plugin tools.
 *
 * A plugin is a command the user chose to expose to the model. It runs with
 * the user's privileges, so it is gated behind the same capability as shell
 * execution and it always asks for approval. The command is an argv list,
 * never a shell string: nothing the model supplies is ever interpolated into
 * a command line.
 */
export function PluginSettings() {
  const [plugins, setPlugins] = useState<PluginView[]>([]);
  const [draft, setDraft] = useState<PluginView>(BLANK());
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let live = true;
    void api.listPlugins()
      .then(value => { if (live) { setPlugins(value); setLoaded(true); } })
      .catch(error => { if (live) { setStatus(errorMessage(error)); setLoaded(true); } });
    return () => { live = false; };
  }, []);

  const save = async () => {
    setBusy(true);
    setStatus('');
    try {
      const saved = await api.savePlugin(draft);
      setPlugins(current => [...current.filter(plugin => plugin.name !== saved.name), saved]);
      setDraft(BLANK());
      setStatus(`Saved ${saved.name}.`);
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (name: string) => {
    setBusy(true);
    setStatus('');
    try {
      await api.removePlugin(name);
      setPlugins(current => current.filter(plugin => plugin.name !== name));
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const commandLine = draft.command.join(' ');

  if (!loaded) return <section className="settings-section"><p className="panel-empty">Loading plugins…</p></section>;

  return <section className="settings-section plugin-settings">
    <h2>Plugin tools</h2>
    <p className="settings-hint">
      A plugin exposes one command to the model. It runs with your privileges, so it always asks for approval and
      it is disabled together with shell execution. The command is an argument list — the model never supplies
      shell text.
    </p>
    {plugins.length === 0
      ? <p className="panel-empty">No plugins installed.</p>
      : <ul className="plugin-list">
        {plugins.map(plugin => <li key={plugin.name} className={plugin.status === 'error' ? 'invalid' : ''}>
          <div>
            <strong>{plugin.name || plugin.path.split('/').pop()}</strong>
            {plugin.status === 'error'
              ? <p className="settings-error">{plugin.error}</p>
              : <p className="settings-hint">{plugin.description || 'No description.'}</p>}
            {plugin.status === 'ok' && <code className="plugin-command">{plugin.command.join(' ')}</code>}
          </div>
          {plugin.status === 'ok' && <button className="icon-button" aria-label={`Remove plugin ${plugin.name}`} disabled={busy} onClick={() => void remove(plugin.name)}><Trash2 size={15} /></button>}
        </li>)}
      </ul>}
    <h3>Add a plugin</h3>
    <div className="plugin-form">
      <label>Name<input value={draft.name} placeholder="deploy" onChange={event => setDraft(current => ({ ...current, name: event.target.value }))} /></label>
      <label>Description<input value={draft.description} placeholder="Deploy the current build to staging." onChange={event => setDraft(current => ({ ...current, description: event.target.value }))} /></label>
      <label>Command (argument list)<input value={commandLine} placeholder="bash scripts/deploy.sh" onChange={event => setDraft(current => ({ ...current, command: event.target.value.trim() ? event.target.value.trim().split(/\s+/) : [] }))} /></label>
      <label>Timeout (seconds)<input type="number" min={1} max={600} value={draft.timeoutSeconds} onChange={event => setDraft(current => ({ ...current, timeoutSeconds: Number(event.target.value) || 60 }))} /></label>
      <button className="primary" disabled={busy || !draft.name.trim() || !draft.command.length} onClick={() => void save()}><Plus size={15} /> Save plugin</button>
    </div>
    {status && <p className="settings-status" role="status">{status}</p>}
  </section>;
}
