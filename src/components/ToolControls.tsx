import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ConnectorView, ToolApproval } from '../lib/types';

export function ToolPicker({ selected, onChange, busy }: { selected: string[]; onChange: (ids: string[]) => void; busy: boolean }) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.listConnectors().then(value => { if (!disposed) setItems(value.filter(item => item.connected)); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  return <details className="tool-picker"><summary>Tools · {selected.length ? `${selected.length} connectors selected` : 'Off'}</summary>
    <p>Selected tools may send data to their services. Each action requires your approval.</p>
    {items.map(item => <label key={item.id}><input type="checkbox" checked={selected.includes(item.id)} disabled={busy} onChange={event => onChange(event.target.checked ? [...selected, item.id] : selected.filter(id => id !== item.id))} />{item.id}<small>{item.tools.length} tools</small></label>)}
    {!items.length && <p>Connect a service in Connectors to make its tools available here.</p>}
    {error && <p role="alert">{error}</p>}
  </details>;
}

export function ApprovalDialog({ request, onResolve }: { request: ToolApproval; onResolve: (allow: boolean) => Promise<void> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { dialog.current?.showModal(); }, []);
  async function resolve(allow: boolean) {
    setBusy(true); setError('');
    try { await onResolve(allow); }
    catch (e) { setError(errorMessage(e)); setBusy(false); }
  }
  return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void resolve(false); }}>
    <p className="eyebrow">ACTION REQUEST</p><h2 id="approval-title">Allow {request.connector} to run this tool?</h2>
    <p><strong>{request.name}</strong></p><p>The arguments below will be sent to this service. The action may read or change external data.</p>
    <pre>{JSON.stringify(request.arguments, null, 2)}</pre>
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
