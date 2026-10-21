import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { LspConfig, LspServerConfig } from '../lib/types';

type Draft = Omit<LspServerConfig, 'arguments' | 'environment' | 'initializationOptions' | 'settings'> & {
  arguments: string; environment: string; initializationOptions: string; settings: string;
};
const toDraft = (server: LspServerConfig): Draft => ({ ...server,
  arguments: JSON.stringify(server.arguments, null, 2), environment: JSON.stringify(server.environment, null, 2),
  initializationOptions: JSON.stringify(server.initializationOptions, null, 2), settings: JSON.stringify(server.settings, null, 2),
});
function parse(server: Draft): LspServerConfig {
  function json(field: 'arguments' | 'environment' | 'initializationOptions' | 'settings'): unknown {
    try { return JSON.parse(server[field]); } catch { throw new Error(`${server.id || 'Server'}: ${field} must be valid JSON.`); }
  }
  const args = json('arguments'); const env = json('environment');
  if (!Array.isArray(args) || !args.every(value => typeof value === 'string')) throw new Error(`${server.id}: arguments must be a JSON array of strings.`);
  if (!env || typeof env !== 'object' || Array.isArray(env) || !Object.values(env).every(value => typeof value === 'string')) throw new Error(`${server.id}: environment must be a JSON object with string values.`);
  return { ...server, arguments: args, environment: env as Record<string, string>, initializationOptions: json('initializationOptions'), settings: json('settings') };
}

export function LanguageServerSettings({ root }: { root?: string }) {
  const [config, setConfig] = useState<LspConfig | null>(null);
  const [servers, setServers] = useState<Draft[]>([]);
  const [busy, setBusy] = useState(nativeAvailable);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getLspConfig().then(value => { if (!disposed) { setConfig(value); setServers(value.servers.map(toDraft)); } })
      .catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, []);
  function changed() { setDirty(true); setNotice(''); setError(''); }
  function change(index: number, value: Partial<Draft>) {
    changed(); setServers(current => current.map((server, i) => i === index ? { ...server, ...value } : server));
  }
  async function action(run: () => Promise<void>) {
    setBusy(true); setError(''); setNotice('');
    try { await run(); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  async function save() {
    if (!config) return;
    const next = { ...config, servers: servers.map(parse) };
    const ids = new Set<string>();
    for (const server of next.servers) {
      if (!/^[A-Za-z0-9_.-]{1,64}$/.test(server.id) || ids.has(server.id)) throw new Error('Use a unique server ID with 1–64 letters, numbers, dots, dashes or underscores.');
      ids.add(server.id);
      if (!server.languageId.trim() || !server.command.trim()) throw new Error(`${server.id}: enter a language ID and an absolute executable path.`);
      if (!Number.isInteger(server.timeoutSeconds) || server.timeoutSeconds < 1 || server.timeoutSeconds > 120) throw new Error(`${server.id}: timeout must be a whole number from 1 to 120 seconds.`);
    }
    await api.saveLspConfig(next);
    setConfig(next); setDirty(false); setNotice('Language server settings saved. New tool calls use these settings.');
  }
  return <div className="settings-page">
    <div className="page-heading"><h1>Language servers</h1><p>Use installed language servers for diagnostics, symbols, references and rename previews.</p></div>
    {!nativeAvailable && <p className="catalog-notice">Open the desktop application to configure language servers.</p>}
    <p className="catalog-notice">Choose executables you trust. Servers run with your account’s permissions in the selected workspace. Nothing is installed automatically.</p>
    <form onSubmit={event => { event.preventDefault(); void action(save); }}>
      <fieldset disabled={busy || !nativeAvailable || !config}>
        <section className="preference-section" aria-labelledby="lsp-policy-heading">
          <div className="preference-section-heading"><h2 id="lsp-policy-heading">Availability</h2></div>
          <div className="preference-group">
            <div className="preference-row preference-toggle"><div className="preference-copy"><label htmlFor="lsp-enabled">Enable language server tools</label><p>Offer configured servers when the tools are selected for a conversation.</p></div><input id="lsp-enabled" className="switch" type="checkbox" checked={config?.enabled ?? false} onChange={event => { changed(); setConfig(current => current && ({ ...current, enabled: event.target.checked })); }} /></div>
            <div className="preference-row preference-toggle"><div className="preference-copy"><label htmlFor="lsp-always-ask">Always request approval</label><p>Ask before every tool call, including in Full access.</p></div><input id="lsp-always-ask" className="switch" type="checkbox" checked={config?.alwaysAsk ?? false} onChange={event => { changed(); setConfig(current => current && ({ ...current, alwaysAsk: event.target.checked })); }} /></div>
            <div className="settings-note"><p>Rename tools preview changes before applying them. Application checks the reviewed files have not changed.</p></div>
          </div>
        </section>
        {servers.map((server, index) => {
          const prefix = `lsp-${index}`;
          return <section className="preference-section" key={index} aria-labelledby={`${prefix}-heading`}>
            <div className="preference-section-heading"><h2 id={`${prefix}-heading`}>{server.id || 'New server'}</h2></div>
            <div className="preference-group">
              <div className="preference-row preference-toggle"><div className="preference-copy"><label htmlFor={`${prefix}-enabled`}>Enable this server</label></div><input id={`${prefix}-enabled`} className="switch" type="checkbox" checked={server.enabled} onChange={event => change(index, { enabled: event.target.checked })} /></div>
              <div className="preference-row preference-row-block"><div className="field-pair">
                <label htmlFor={`${prefix}-id`}>Server ID<input id={`${prefix}-id`} required maxLength={64} value={server.id} onChange={event => change(index, { id: event.target.value })} /></label>
                <label htmlFor={`${prefix}-language`}>Language ID<input id={`${prefix}-language`} required maxLength={64} placeholder="For example: typescript" value={server.languageId} onChange={event => change(index, { languageId: event.target.value })} /></label>
              </div></div>
              <div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor={`${prefix}-command`}>Server executable</label><p>Absolute path to an installed executable. Supply arguments separately.</p></div>
                <div className="field-row"><input id={`${prefix}-command`} required value={server.command} onChange={event => change(index, { command: event.target.value })} /><button type="button" className="secondary" aria-label={`Browse executable for ${server.id || 'new server'}`} onClick={() => void action(async () => { const path = await open({ multiple: false }); if (typeof path === 'string') change(index, { command: path }); })}>Browse</button></div>
              </div>
              <div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor={`${prefix}-arguments`}>Arguments</label><p>A JSON array of strings, for example {`["--stdio"]`}. No shell expansion.</p></div><textarea id={`${prefix}-arguments`} rows={3} spellCheck={false} value={server.arguments} onChange={event => change(index, { arguments: event.target.value })} /></div>
              <div className="preference-row preference-row-block"><label htmlFor={`${prefix}-timeout`}>Request timeout (seconds)</label><input id={`${prefix}-timeout`} type="number" required min={1} max={120} step={1} value={server.timeoutSeconds} onChange={event => change(index, { timeoutSeconds: Number(event.target.value) })} /></div>
              <div className="preference-row preference-row-block"><details><summary>Advanced configuration</summary>
                <div className="preference-row preference-row-block"><label htmlFor={`${prefix}-commands`}>Allowed server commands</label><p className="preference-desc">One command name per line. Executing a command requires approval and may change files directly.</p><textarea id={`${prefix}-commands`} rows={3} value={(server.allowedCommands ?? []).join('\n')} onChange={event => change(index, { allowedCommands: event.target.value.split('\n').map(v => v.trim()).filter(Boolean) })} /></div>
                {(['environment', 'initializationOptions', 'settings'] as const).map(field => <div className="preference-row preference-row-block" key={field}><label htmlFor={`${prefix}-${field}`}>{field === 'environment' ? 'Environment variables' : field === 'initializationOptions' ? 'Initialization options' : 'Server settings'}</label>{field === 'environment' && <p className="preference-desc">JSON string values. Stored on this device; use the credential vault for secrets.</p>}<textarea id={`${prefix}-${field}`} rows={4} spellCheck={false} value={server[field]} onChange={event => change(index, { [field]: event.target.value })} /></div>)}
              </details></div>
              <div className="connector-actions"><button type="button" className="secondary" disabled={dirty || !root || !config?.enabled || !server.enabled} aria-describedby="lsp-test-help" onClick={() => void action(async () => { const result = await api.testLspServer(server.id, root!); if (!result.ready) throw new Error(result.message || 'Language server did not initialize.'); setNotice(result.message || 'Language server initialized.'); })}>Test {server.id || 'server'}</button><button type="button" className="secondary" aria-label={`Remove ${server.id || 'new server'}`} onClick={() => { changed(); setServers(current => current.filter((_, i) => i !== index)); }}>Remove</button></div>
            </div>
          </section>;
        })}
        <div className="connector-actions"><button type="button" className="secondary" disabled={servers.length >= 8} onClick={() => { let n = 1; while (servers.some(server => server.id === `server-${n}`)) n++; changed(); setServers(current => [...current, toDraft({ id: `server-${n}`, languageId: '', command: '', enabled: true, arguments: [], environment: {}, initializationOptions: null, settings: null, timeoutSeconds: 30 })]); }}>Add server</button><button className="primary" disabled={!dirty}>{busy ? 'Working…' : 'Save language server settings'}</button></div>
        {!servers.length && <p className="preference-desc">No servers configured. Add an installed server to begin.</p>}
        <p id="lsp-test-help" className="preference-desc">Save changes before testing. Testing starts the saved executable in the current conversation’s workspace.{!root && ' Choose a workspace in the conversation to enable Test.'}{!config?.enabled && ' Enable language server tools to test a server.'}</p>
      </fieldset>
    </form>
    {busy && <p role="status">Loading or checking language server settings…</p>}
    {dirty && <p role="status">Unsaved changes.</p>}
    {error && <p className="error-banner" role="alert">{error}</p>}
    {notice && <p role="status">{notice}</p>}
  </div>;
}
