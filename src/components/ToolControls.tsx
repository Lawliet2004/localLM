import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, ConnectorView, ToolApproval, ToolSelection } from '../lib/types';

const connectorName = (item: ConnectorView) => (item.authType === 'local' ? item.description : item.id);
const workspaceName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() || path;

interface ToolPickerProps {
  selected: string[]; onChange: (ids: string[]) => void;
  selectedTools: ToolSelection[]; onToolsChange: (tools: ToolSelection[]) => void;
  accessMode?: AccessMode; onAccessModeChange?: (mode: AccessMode) => void;
  busy: boolean; loading?: boolean; saving?: boolean;
  onNavigate?: (page: 'connectors' | 'execution' | 'skills') => void;
}

export function ToolPicker({ selected, onChange, selectedTools, onToolsChange, accessMode = 'ask', onAccessModeChange, busy, loading = false, saving = false, onNavigate }: ToolPickerProps) {
  const picker = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    function dismiss(event: PointerEvent) { if (picker.current && !picker.current.contains(event.target as Node)) picker.current.open = false; }
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, []);
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [hasDaytona, setHasDaytona] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [reconnecting, setReconnecting] = useState('');
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [pending, setPending] = useState(true);
  const query = search.toLowerCase();
  useEffect(() => {
    if (!nativeAvailable) { setPending(false); return; }
    let disposed = false;
    let remaining = 4;
    const settle = () => { remaining -= 1; if (remaining === 0 && !disposed) setPending(false); };
    const fail = (e: unknown) => { if (!disposed) setError(current => current || errorMessage(e)); };
    api.hasDaytonaKey().then(value => { if (!disposed) setHasDaytona(value); }).catch(fail).finally(settle);
    api.listConnectors().then(value => { if (!disposed) setItems(value); }).catch(fail).finally(settle);
    api.listSkills().then(value => { if (!disposed) setActiveSkills(value.filter(item => item.active).map(item => item.id)); }).catch(fail).finally(settle);
    api.getWorkspace().then(value => { if (!disposed) setWorkspace(value.path); }).catch(fail).finally(settle);
    return () => { disposed = true; };
  }, []);
  const isSelected = (connectorId: string, toolName: string) => selectedTools.some(tool => tool.connectorId === connectorId && tool.toolName === toolName);
  const toolCount = selectedTools.length + (selected.includes('__workspace') ? 5 : 0) + (selected.includes('__execution') ? 1 : 0) + (selected.includes('__daytona') ? 1 : 0) + (activeSkills.length ? 1 : 0);
  // Selection is independent of availability: a failed discovery or a
  // disconnected service never removes a saved choice.
  const connectedTools = new Map(items.filter(item => item.connected).map(item => [item.id, new Set(item.tools.map(tool => tool.name))]));
  const unavailableTools = pending ? [] : selectedTools.filter(tool => !connectedTools.get(tool.connectorId)?.has(tool.toolName));
  const workspaceMissing = !pending && !workspace;
  const daytonaMissing = !pending && !hasDaytona;
  const unavailableCount = unavailableTools.length
    + (selected.includes('__workspace') && workspaceMissing ? 1 : 0)
    + (selected.includes('__execution') && workspaceMissing ? 1 : 0)
    + (selected.includes('__daytona') && daytonaMissing ? 1 : 0);
  const summary = loading ? 'Tools · Loading…' : toolCount ? `Tools · ${toolCount}/32 enabled` : 'Tools · Off';
  const summaryDetails = `${unavailableCount ? ` · ${unavailableCount} unavailable` : ''}${activeSkills.length ? ` · ${activeSkills.length} active skills` : ''}${saving ? ' · Saving…' : ''}`;
  function toggleTools(item: ConnectorView, names: string[], enabled: boolean) {
    const remaining = selectedTools.filter(tool => tool.connectorId !== item.id || !names.includes(tool.toolName));
    const next = enabled ? [...remaining, ...names.map(toolName => ({ connectorId: item.id, toolName }))] : remaining;
    if (toolCount - selectedTools.length + next.length > 32) { setError('Choose at most 32 tools, including workspace, execution and skill reading tools.'); return; }
    setError(''); onToolsChange(next);
  }
  async function reconnect(item: ConnectorView) {
    setReconnecting(item.id); setError('');
    try {
      const connected = await api.connectConnector(item.id);
      setItems(current => current.map(value => value.id === item.id ? connected : value));
    } catch (e) { setError(`${connectorName(item)} could not reconnect: ${errorMessage(e)}`); }
    finally { setReconnecting(''); }
  }
  async function chooseWorkspace() {
    setChoosing(true); setError('');
    try { const path = await open({ directory: true, multiple: false, title: 'Choose workspace folder' }); if (typeof path === 'string') { await api.setWorkspace(path); setWorkspace(path); } }
    catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(false); }
  }
  return <><div className={`permission-bar permission-${accessMode}`}><label>Permissions<select aria-label="Permission mode" value={accessMode} disabled={!nativeAvailable || busy || !onAccessModeChange} onChange={event => onAccessModeChange?.(event.target.value as AccessMode)}><option value="ask">Ask for approval</option><option value="autoApprove">Auto-approve reads</option><option value="fullAccess">Full access</option></select></label><span>{accessMode === 'fullAccess' ? 'Selected tools run without prompts, including code and external changes.' : accessMode === 'autoApprove' ? 'Workspace reads run automatically. Other actions ask.' : 'Every tool action asks first.'}</span></div>
  <details ref={picker} className="tool-picker" onKeyDown={event => { if (event.key === 'Enter' && event.target instanceof HTMLInputElement && event.target.type === 'search') event.preventDefault(); if (event.key === 'Escape' && picker.current) { picker.current.open = false; picker.current.querySelector('summary')?.focus(); } }}><summary>{summary}{summaryDetails}</summary><div className="tool-picker-body">
    <p className="tool-picker-note">Tool choices are remembered for new chats. Existing chats keep their own selections. Selected tools may send data to their services, and permissions apply to this conversation and are recorded with each action.</p>
    {activeSkills.length > 0 && <p>Active skills: {activeSkills.join(', ')}. One tool enables reading their package files. Deactivate skills in Skills to remove it.</p>}
    <div className="tool-group"><h4>Workspace</h4>
      <label><input type="checkbox" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || !workspace || (!selected.includes('__workspace') && toolCount + 5 > 32)} onChange={event => onChange(event.target.checked ? [...selected, '__workspace'] : selected.filter(id => id !== '__workspace'))} />Workspace files</label>
      <button type="button" className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{workspace ? 'Change folder' : 'Choose folder'}</button>
      {workspace ? <span className="workspace-choice"><strong>{workspaceName(workspace)}</strong><details className="workspace-path"><summary>Full path</summary><code>{workspace}</code></details></span> : <small>{pending ? 'Checking for a workspace folder…' : 'Choose a folder to let the assistant read and edit files there.'}</small>}
    </div>
    <div className="tool-group"><h4>Code execution</h4>
      <label><input type="checkbox" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || !workspace || (!selected.includes('__execution') && toolCount >= 32)} onChange={event => onChange(event.target.checked ? [...selected, '__execution'] : selected.filter(id => id !== '__execution'))} />Local code <small>Not sandboxed · {accessMode === 'fullAccess' ? 'runs without prompts' : 'approval required'}</small></label>
      {selected.includes('__execution') && workspaceMissing && <p className="tool-unavailable" role="status">Selected but unavailable: choose a workspace folder above.</p>}
      <label><input type="checkbox" aria-label="Daytona cloud code" checked={selected.includes('__daytona')} disabled={busy || (!selected.includes('__daytona') && (!hasDaytona || toolCount >= 32))} onChange={event => onChange(event.target.checked ? [...selected, '__daytona'] : selected.filter(id => id !== '__daytona'))} />Daytona cloud code <small>{hasDaytona ? 'Remote sandbox · usage may incur charges' : 'Save a Daytona key in Execution'}</small></label>
      {selected.includes('__daytona') && daytonaMissing && <p className="tool-unavailable" role="status">Selected but unavailable: save a Daytona key in Execution. {onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('execution')}>Open Execution</button>}</p>}
    </div>
    <div className="tool-group"><h4>Connected services</h4>
      {items.length > 0 && <input type="search" aria-label="Search available tools" placeholder="Find a tool or connector…" value={search} onChange={event => setSearch(event.target.value)} />}
      <div className="connector-tool-groups">{items.map(item => {
        const name = connectorName(item);
        const selectedFor = selectedTools.filter(tool => tool.connectorId === item.id);
        if (!item.connected) {
          if (!(`${item.id} ${item.description}`.toLowerCase().includes(query) || selectedFor.length)) return null;
          return <div key={item.id} className="connector-tool-group connector-offline"><p><strong>{name}</strong> <small>Not connected</small></p>
            {selectedFor.length > 0 && <p className="tool-unavailable" role="status">{selectedFor.length} selected {selectedFor.length === 1 ? 'tool stays' : 'tools stay'} selected and recover when the service reconnects.</p>}
            <button type="button" className="secondary" disabled={busy || reconnecting === item.id} onClick={() => void reconnect(item)}>{reconnecting === item.id ? 'Reconnecting…' : 'Reconnect'}</button></div>;
        }
        const visible = item.tools.filter(tool => `${item.id} ${name} ${tool.name} ${tool.description}`.toLowerCase().includes(query));
        const missing = selectedFor.filter(tool => !item.tools.some(available => available.name === tool.toolName));
        if (!visible.length && !missing.length) return null;
        const count = item.tools.filter(tool => isSelected(item.id, tool.name)).length;
        return <details key={item.id} className="connector-tool-group"><summary>{name} <small>{count}/{item.tools.length} selected</small></summary>
          <label><input type="checkbox" aria-label={name} checked={count === item.tools.length && count > 0} disabled={busy} onChange={event => toggleTools(item, item.tools.map(tool => tool.name), event.target.checked)} />All {name} tools</label>
          {visible.map(tool => <label key={tool.name} className="connector-tool-choice"><input type="checkbox" aria-label={tool.name} checked={isSelected(item.id, tool.name)} disabled={busy || (!isSelected(item.id, tool.name) && toolCount >= 32)} onChange={event => toggleTools(item, [tool.name], event.target.checked)} /><span><strong>{tool.name}</strong><small>{tool.description || 'No description provided.'}</small></span></label>)}
          {missing.map(tool => <div key={tool.toolName} className="connector-tool-missing"><span><strong>{tool.toolName}</strong><small>No longer offered by {name}. The selection is kept until you remove it.</small></span><button type="button" className="secondary" disabled={busy} onClick={() => onToolsChange(selectedTools.filter(other => !(other.connectorId === item.id && other.toolName === tool.toolName)))}>Remove</button></div>)}
        </details>;
      })}</div>
      {!items.length && (pending ? <p>Looking for connectors…</p> : <p>Connect a service in Connectors to make its tools available here.{onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('connectors')}>Open Connectors</button>}</p>)}
    </div>
    {unavailableTools.length > 0 && <div className="unavailable-tools" role="status"><p>Selected but currently unavailable. These selections are kept and recover automatically when their service does:</p>
      <ul>{unavailableTools.map(tool => {
        const item = items.find(value => value.id === tool.connectorId);
        const reason = !item ? 'connector is no longer configured' : item.connected ? 'tool is no longer offered' : 'service is not connected';
        return <li key={`${tool.connectorId}/${tool.toolName}`}><strong>{tool.toolName}</strong><small>{reason} · {item ? connectorName(item) : tool.connectorId}</small></li>;
      })}</ul>
      <button type="button" className="secondary" disabled={busy} onClick={() => onToolsChange(selectedTools.filter(tool => connectedTools.get(tool.connectorId)?.has(tool.toolName)))}>Remove unavailable tools</button></div>}
    {error && <p role="alert">{error}</p>}
  </div></details></>;
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
    <p className="eyebrow">ACTION REQUEST</p><h2 id="approval-title">{request.localServerName ? `Allow local server ${request.localServerName} to run this tool?` : request.connector === 'Local execution' ? 'Run this code on your computer?' : `Allow ${request.connector} to run this tool?`}</h2>
    {request.localServerName && <p>This local server runs with your account’s file and network permissions. Review the arguments before allowing this action. Server ID: <code>{request.connector}</code></p>}
    <p><strong>{request.name}</strong></p><p>{request.connector === 'Local execution' ? 'This code runs with your Windows account’s permissions. It can access files outside the workspace and the network. It is not sandboxed. Review the complete code before allowing it.' : request.connector === 'Daytona' ? 'This code will be sent to a temporary Daytona cloud sandbox and may incur usage charges. No local files are uploaded automatically. Cleanup is attempted after execution.' : request.connector === 'Workspace' ? 'This action will access the workspace folder on your computer using the arguments below.' : request.connector === 'Skills' ? 'Read a verified file from an active skill’s local package. This does not execute scripts or contact a service.' : 'The arguments below will be sent to this service. The action may read or change external data.'}</p>
    {(request.connector === 'Local execution' || request.connector === 'Daytona') && typeof request.arguments.code === 'string' ? <><p>Language: {String(request.arguments.language)} · Timeout: {String(request.arguments.timeout_seconds ?? 60)} seconds</p><pre className="approval-code">{request.arguments.code}</pre><details><summary>Full request</summary><pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre></details></> : <pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre>}
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button type="button" autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button type="button" className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
