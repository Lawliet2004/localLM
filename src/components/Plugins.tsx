import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Blocks, FolderPlus, Trash2 } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { InstalledPlugin } from '../lib/types';

export function Plugins() {
  const [items, setItems] = useState<InstalledPlugin[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [inspection, setInspection] = useState('');
  async function act(action: () => Promise<unknown>) {
    setBusy(true); setError('');
    try { await action(); setItems(await api.listPlugins()); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { if (nativeAvailable) void act(async () => {}); }, []);
  async function install() {
    const path = await open({ directory: true, title: 'Choose a LocalLM plugin folder' });
    if (typeof path === 'string') {
      const scan = await api.scanPlugin(path);
      setInspection(JSON.stringify(scan, null, 2));
      await api.installPlugin(path);
    }
  }
  return <section className="management-page">
    <div className="page-heading"><div><Blocks size={24} /><h1>Plugin Marketplace</h1><p>Manage your LocalLM extensions.</p></div><button className="primary" disabled={busy || !nativeAvailable} onClick={() => void act(install)}><FolderPlus size={16} /> Install from folder</button></div>
    <p className="muted">LocalLM plugins contribute configuration, presets, and instructions. Choose a folder containing plugin.json. Skills and service connections remain available in their own settings.</p>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {!items.length && <div className="empty-panel">No plugins installed.</div>}
    {items.map(item => <article key={item.name} className="management-item">
      <div><strong>{item.name}</strong><p>Version {item.version}</p><small>{item.path}</small></div>
      <div className="button-row"><label><input type="checkbox" checked={item.enabled} disabled={busy} onChange={e => void act(() => api.setPluginEnabled(item.name, e.target.checked))} /> Enabled</label>
        <button disabled={busy} onClick={() => void act(async () => setInspection(JSON.stringify(await api.testPlugin(item.path), null, 2)))}>Inspect</button>
        <button disabled={busy} aria-label={`Remove ${item.name}`} onClick={() => void act(() => api.removePlugin(item.name))}><Trash2 size={15} /></button></div>
    </article>)}
    {inspection && <details open><summary>Plugin inspection</summary><pre>{inspection}</pre></details>}
  </section>;
}
