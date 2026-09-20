import { useEffect, useState } from 'react';
import { Plug, Search } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import catalog from '../lib/catalog.json';
import type { ConnectorView } from '../lib/types';
import { LocalConnectorForm } from './LocalConnectorForm';
import type { LocalServerConfig } from '../lib/api';

const presets: ConnectorView[] = catalog.connectors.map(item => ({
  id: item.name, description: item.description, url: item.url,
  authType: item.auth?.type === 'dcr' ? 'oauth' : item.auth?.type === 'header' ? 'apiKey' : 'none',
  connected: false, hasCredential: false, tools: [],
}));

export function Connectors() {
  const [items, setItems] = useState(presets);
  const [query, setQuery] = useState('');
  const [pending, setPending] = useState('');
  const [signingIn, setSigningIn] = useState(false);
  const [tokens, setTokens] = useState<Record<string, string>>({});
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<LocalServerConfig>();
  async function editLocal(item: ConnectorView) {
    setPending(item.id); setError('');
    try { setEditing(await api.readLocalConnector(item.id)); }
    catch (e) { setError(errorMessage(e)); }
    finally { setPending(''); }
  }
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.listConnectors().then(value => { if (!disposed) setItems(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function connect(item: ConnectorView, signIn = false) {
    setPending(item.id); setSigningIn(signIn); setError('');
    const token = tokens[item.id];
    setTokens(current => ({ ...current, [item.id]: '' }));
    try {
      const connected = await (signIn ? api.signInConnector(item.id) : api.connectConnector(item.id, token || undefined));
      setItems(current => current.map(value => value.id === item.id ? connected : value));
    } catch (e) { setError(errorMessage(e)); }
    finally { setPending(''); setSigningIn(false); }
  }
  async function disconnect(item: ConnectorView, forget: boolean) {
    setPending(item.id); setError('');
    try { await api.disconnectConnector(item.id, forget); setItems(await api.listConnectors()); }
    catch (e) { setError(errorMessage(e)); }
    finally { setPending(''); }
  }
  async function removeLocal(item: ConnectorView) {
    setPending(item.id); setError('');
    try { await api.disconnectConnector(item.id, false); await api.removeLocalConnector(item.id); setItems(await api.listConnectors()); }
    catch (e) { setError(errorMessage(e)); }
    finally { setPending(''); }
  }
  const filtered = items.filter(item => `${item.id} ${item.description}`.toLowerCase().includes(query.toLowerCase()));
  return <div className="settings-page catalog-page">
    <div className="page-heading"><p className="eyebrow">EXTEND YOUR WORKSPACE</p><h1>Connectors</h1><p>Connect your accounts and discover the tools they provide.</p></div>
    <div className="catalog-toolbar"><label className="search-field"><Search size={16} /><input type="search" aria-label="Search connectors" placeholder="Search connectors…" value={query} onChange={e => setQuery(e.target.value)} /></label><span>{filtered.length} connectors</span></div>
    <p className="catalog-notice">Credentials are encrypted on this device. Connected services reconnect when you reopen the app; disconnect to keep a service off. Services that need renewed authorization may ask you to sign in again. Actions follow your conversation's permission mode. Connecting a remote service sends tool arguments to that service; it never runs code on this device.</p>
    <LocalConnectorForm key={editing?.id ?? 'new'} initial={editing} onCancel={editing ? () => setEditing(undefined) : undefined} onSaved={async () => { setEditing(undefined); setItems(await api.listConnectors()); }} />
    {error && <p role="alert" className="error-banner">{error}</p>}
    <div className="catalog-list">{filtered.map(item => <details className="catalog-item" key={item.id}>
      <summary><span className="catalog-icon"><Plug size={18} /></span><span><strong>{item.authType === 'local' ? item.description : item.id.split('-').map(word => word.charAt(0).toUpperCase() + word.slice(1)).join(' ')}</strong><small>{item.authType === 'local' ? 'Local MCP server' : item.description}</small></span><small>{item.connected ? `${item.tools.length} tools` : 'Not connected'}</small></summary>
      <div className="catalog-detail">{item.authType === 'local' ? <p>Connect launches this program with your account's file and network permissions. Disconnect stops its process tree.</p> : <><span>MCP service</span><code>{item.url}</code><small>{item.authType === 'oauth' ? 'OAuth sign-in with automatic registration; tokens refresh, expire, revoke, and reconnect through the same flow.' : item.authType === 'apiKey' ? 'Bearer API token stored encrypted; replace or forget it here. It is sent only to this service endpoint.' : 'Public endpoint; no account credential is stored. Arguments you approve are still sent to this service.'}</small></>}
        {item.connectionError && <p className="error-banner" role="alert">Could not reconnect: {item.connectionError}</p>}
        <form onSubmit={event => { event.preventDefault(); void connect(item, item.authType === 'oauth' && !item.hasCredential); }}>
          {item.authType === 'apiKey' && !item.connected && <label>API token<input type="password" aria-label={`${item.id} API token`} autoComplete="off" value={tokens[item.id] || ''} placeholder={item.hasCredential ? 'Saved token · enter to replace' : 'Enter your API token'} onChange={event => setTokens(current => ({ ...current, [item.id]: event.target.value }))} /></label>}
          <div className="connector-actions">
            {item.authType === 'local' && !item.connected && <button type="button" className="secondary" disabled={!nativeAvailable || !!pending} onClick={() => void editLocal(item)}>Edit configuration</button>}
            {!item.connected && <button className="primary" disabled={!nativeAvailable || !!pending || (item.authType === 'apiKey' && !item.hasCredential && !tokens[item.id])}>{pending === item.id ? signingIn ? 'Waiting for sign-in…' : 'Connecting…' : item.authType === 'oauth' && !item.hasCredential ? 'Sign in' : 'Connect'}</button>}
            {(item.connected || item.connectionError) && <button type="button" className="secondary" disabled={!!pending} onClick={() => void disconnect(item, false)}>Disconnect</button>}
            {item.authType === 'local' && !item.connected && <><button type="button" className="secondary" disabled={!nativeAvailable || !!pending} onClick={() => void disconnect(item, false)}>Stop session</button><button type="button" className="secondary" disabled={!nativeAvailable || !!pending} onClick={() => void removeLocal(item)}>Remove server</button></>}
            {item.hasCredential && <button type="button" className="secondary" disabled={!!pending} onClick={() => void disconnect(item, true)}>Forget credentials</button>}
            {item.authType === 'oauth' && item.hasCredential && <button type="button" className="secondary" disabled={!!pending} onClick={() => void connect(item, true)}>Sign in again</button>}
            {pending === item.id && signingIn && <button type="button" className="secondary" onClick={() => void api.cancelConnectorSignIn().catch(e => setError(errorMessage(e)))}>Cancel sign-in</button>}
          </div>
        </form>
        {item.connected && <div className="connector-tools">{item.tools.map(tool => <details key={tool.name}><summary>{tool.name}</summary><p>{tool.description}</p><pre>{JSON.stringify(tool.inputSchema, null, 2)}</pre></details>)}</div>}
      </div>
    </details>)}</div>
    {!filtered.length && <div className="empty-state">No connectors match “{query}”.</div>}
  </div>;
}

