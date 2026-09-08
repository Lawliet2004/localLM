import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ConnectorView, ToolApproval } from '../lib/types';

export function ToolPicker({ selected, onChange, busy }: { selected: string[]; onChange: (ids: string[]) => void; busy: boolean }) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.listConnectors().then(value => { if (!disposed) setItems(value.filter(item => item.connected)); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.listSkills().then(value => { if (!disposed) setActiveSkills(value.filter(item => item.active).map(item => item.id)); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.getWorkspace().then(value => { if (!disposed) setWorkspace(value.path); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function chooseWorkspace() {
    setChoosing(true); setError('');
    try { const path = await open({ directory: true, multiple: false, title: 'Choose workspace folder' }); if (typeof path === 'string') { await api.setWorkspace(path); setWorkspace(path); } }
    catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(false); }
  }
  return <details className="tool-picker"><summary>Tools · {selected.length ? `${selected.length} enabled` : 'Off'}{activeSkills.length > 0 && ` · ${activeSkills.length} active skills`}</summary>
    {activeSkills.length > 0 && <p>Skill guidance: {activeSkills.join(', ')}</p>}
    <p>Selected tools may send data to their services. Each action requires your approval.</p>
    <div className="workspace-tools"><label><input type="checkbox" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || !workspace} onChange={event => onChange(event.target.checked ? [...selected, '__workspace'] : selected.filter(id => id !== '__workspace'))} />Workspace files</label><button className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{workspace ? 'Change folder' : 'Choose folder'}</button>{workspace && <code>{workspace}</code>}</div>
    <label><input type="checkbox" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || !workspace} onChange={event => onChange(event.target.checked ? [...selected, '__execution'] : selected.filter(id => id !== '__execution'))} />Local code <small>Not sandboxed · approval required</small></label>
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
    <p className="eyebrow">ACTION REQUEST</p><h2 id="approval-title">{request.connector === 'Local execution' ? 'Run this code on your computer?' : `Allow ${request.connector} to run this tool?`}</h2>
    <p><strong>{request.name}</strong></p><p>{request.connector === 'Local execution' ? 'This code runs with your Windows account’s permissions. It can access files outside the workspace and the network. It is not sandboxed. Review the complete code before allowing it.' : request.connector === 'Workspace' ? 'This action will access the workspace folder on your computer using the arguments below.' : 'The arguments below will be sent to this service. The action may read or change external data.'}</p>
    {request.connector === 'Local execution' && typeof request.arguments.code === 'string' ? <><p>Language: {String(request.arguments.language)} · Timeout: {String(request.arguments.timeout_seconds ?? 60)} seconds</p><pre className="approval-code">{request.arguments.code}</pre><details><summary>Full request</summary><pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre></details></> : <pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre>}
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
