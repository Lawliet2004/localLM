import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { KvCacheUsage } from '../lib/types';

const mib = (bytes: number) => `${(bytes / 1024 / 1024).toLocaleString(undefined, { maximumFractionDigits: 1 })} MB`;

/** Saved KV slot files (kv_slots.rs): on/off, disk budget, usage and purge. */
export function KvCachePanel() {
  const [usage, setUsage] = useState<KvCacheUsage | null>(null);
  const [budget, setBudget] = useState<number>(4096);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.kvCacheUsage().then(value => { if (!disposed) { setUsage(value); setBudget(value.settings.budgetMb); } })
      .catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function run(action: () => Promise<KvCacheUsage>) {
    setBusy(true); setError('');
    try { const value = await action(); setUsage(value); setBudget(value.settings.budgetMb); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  if (!nativeAvailable) return null;
  const enabled = usage?.settings.enabled ?? true;
  return <section aria-label="Saved KV cache" className="kv-cache-panel">
    <h2>Saved conversation cache</h2>
    <p className="section-description">Saves the local model's processed prompt (KV cache) when a turn finishes, so reopening a long conversation skips most of the prompt processing. The files contain conversation content, stay on this computer, and are deleted with their conversation.</p>
    {error && <p role="alert" className="error-banner">{error}</p>}
    {usage?.runtimeSupported === false && <p role="status">The loaded llama-server does not support slot save/restore, so nothing is being saved.</p>}
    <label className="checkbox-row"><input type="checkbox" aria-label="Save conversation cache" checked={enabled} disabled={busy || !usage}
      onChange={e => { const checked = e.target.checked; void run(() => api.saveKvCacheSettings({ enabled: checked, budgetMb: budget })); }} />Save conversation cache{!enabled && ' (turning it off deletes saved files)'}</label>
    <div className="field-pair">
      <label>Disk budget (MB)<input type="number" aria-label="KV cache disk budget" min={256} max={262144} step={256} value={budget} disabled={busy || !usage}
        onChange={e => setBudget(e.target.valueAsNumber)}
        onBlur={() => { if (usage && Number.isFinite(budget) && budget !== usage.settings.budgetMb) void run(() => api.saveKvCacheSettings({ enabled, budgetMb: budget })); }} /></label>
    </div>
    <div className="form-footer">
      <span role="status">{usage ? `${usage.files} saved · ${mib(usage.bytes)}` : 'Reading…'}</span>
      <button type="button" className="secondary" disabled={busy || !usage || usage.files === 0} onClick={() => void run(api.clearKvCache)}>Delete saved cache</button>
    </div>
  </section>;
}
