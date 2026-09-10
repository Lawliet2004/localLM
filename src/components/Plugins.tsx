import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Plugin, ScanVerdict } from '../lib/types';

export function Plugins() {
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [scan, setScan] = useState<{ path: string; verdict: ScanVerdict } | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(nativeAvailable);
  async function refresh() {
    if (!nativeAvailable) return;
    try { setPlugins(await api.listPlugins()); }
    catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  async function pick(action: 'install' | 'scan' | 'test') {
    setError(''); setNotice(''); setScan(null);
    try {
      const path = await open({ multiple: false, directory: true });
      if (typeof path !== 'string') return;
      if (action === 'scan' || action === 'test') {
        const result = action === 'scan'
          ? await api.scanPlugin(path)
          : (await api.testPlugin(path)).scan as ScanVerdict;
        setScan({ path, verdict: result });
      } else {
        const plugin = await api.installPlugin(path);
        setNotice(`Installed ${plugin.name} ${plugin.version}. Pinned to its folder hash; review before enabling untrusted plugins.`);
        await refresh();
      }
    } catch (e) { setError(errorMessage(e)); }
  }
  async function toggle(plugin: Plugin) {
    setError('');
    try { setPlugins(await api.setPluginEnabled(plugin.name, !plugin.enabled)); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function remove(name: string) {
    setError('');
    try { await api.removePlugin(name); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
  }
  return (
    <div className="settings-page">
      <div className="page-heading"><p className="eyebrow">EXTENSIONS</p><h1>Plugins</h1>
        <p>Plugins contribute configuration the Rust harness interprets; they never execute inside LocalLM. Scan before installing. Experimental: no security audit, least privilege, disposable environments for untrusted work.</p></div>
      {error && <div className="error-banner" role="alert"><span>{error}</span></div>}
      {notice && <div className="preview-banner" role="status">{notice}</div>}
      <div className="row-actions">
        <button type="button" disabled={busy} onClick={() => void pick('install')}>Install from folder</button>
        <button type="button" className="secondary" disabled={busy} onClick={() => void pick('scan')}>Scan folder</button>
        <button type="button" className="secondary" disabled={busy} onClick={() => void pick('test')}>Test in memory</button>
      </div>
      {scan && <div className="scan-result"><p><strong>{scan.verdict.verdict}</strong> · {scan.path}</p>
        <ul>{scan.verdict.findings.map((finding, index) => <li key={index}>{finding}</li>)}</ul></div>}
      <ul className="plugin-list">{plugins.map(plugin => (
        <li key={plugin.name}><span><strong>{plugin.name}</strong> {plugin.version}</span>
          <small>{plugin.enabled ? 'Enabled' : 'Disabled'} · {plugin.path}</small>
          <button type="button" className="secondary" onClick={() => void toggle(plugin)}>{plugin.enabled ? 'Disable' : 'Enable'}</button>
          <button type="button" className="icon-button" aria-label={`Remove ${plugin.name}`} onClick={() => void remove(plugin.name)}>Remove</button></li>
      ))}</ul>
      {!plugins.length && !busy && <p>No plugins installed.</p>}
    </div>
  );
}
