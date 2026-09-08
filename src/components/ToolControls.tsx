import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, ConnectorView, ToolApproval, ToolSelection } from '../lib/types';

export function ToolPicker({ selected, onChange, selectedTools, onToolsChange, accessMode = 'ask', onAccessModeChange, busy }: { selected: string[]; onChange: (ids: string[]) => void; selectedTools: ToolSelection[]; onToolsChange: (tools: ToolSelection[]) => void; accessMode?: AccessMode; onAccessModeChange?: (mode: AccessMode) => void; busy: boolean }) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const toolCount = selectedTools.length + (selected.includes('__workspace') ? 4 : 0) + (selected.includes('__execution') ? 1 : 0);
  const isSelected = (connectorId: string, toolName: string) => selectedTools.some(tool => tool.connectorId === connectorId && tool.toolName === toolName);
  function toggleTools(item: ConnectorView, names: string[], enabled: boolean) {
    const remaining = selectedTools.filter(tool => tool.connectorId !== item.id || !names.includes(tool.toolName));
    const next = enabled ? [...remaining, ...names.map(toolName => ({ connectorId: item.id, toolName }))] : remaining;
    if (toolCount - selectedTools.length + next.length > 32) { setError('Choose at most 32 tools, including workspace and execution tools.'); return; }
    setError(''); onToolsChange(next);
  }
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
  return <><div className={`permission-bar permission-${accessMode}`}><label>Permissions<select aria-label="Permission mode" value={accessMode} disabled={!nativeAvailable || busy || !onAccessModeChange} onChange={event => onAccessModeChange?.(event.target.value as AccessMode)}><option value="ask">Ask for approval</option><option value="autoApprove">Auto-approve reads</option><option value="fullAccess">Full access</option></select></label><span>{accessMode === 'fullAccess' ? 'Selected tools run without prompts, including code and external changes.' : accessMode === 'autoApprove' ? 'Workspace reads run automatically. Other actions ask.' : 'Every tool action asks first.'}</span></div>
  <details className="tool-picker"><summary>Tools · {toolCount ? `${toolCount}/32 enabled` : 'Off'}{activeSkills.length > 0 && ` · ${activeSkills.length} active skills`}</summary>
    {activeSkills.length > 0 && <p>Skill guidance: {activeSkills.join(', ')}</p>}
    <p>Selected tools may send data to their services. Permissions apply to this conversation and are recorded with each action.</p>
    <div className="workspace-tools"><label><input type="checkbox" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || !workspace || (!selected.includes('__workspace') && toolCount + 4 > 32)} onChange={event => onChange(event.target.checked ? [...selected, '__workspace'] : selected.filter(id => id !== '__workspace'))} />Workspace files</label><button className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{workspace ? 'Change folder' : 'Choose folder'}</button>{workspace && <code>{workspace}</code>}</div>
    <label><input type="checkbox" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || !workspace || (!selected.includes('__execution') && toolCount >= 32)} onChange={event => onChange(event.target.checked ? [...selected, '__execution'] : selected.filter(id => id !== '__execution'))} />Local code <small>Not sandboxed · {accessMode === 'fullAccess' ? 'runs without prompts' : 'approval required'}</small></label>
    {items.length > 0 && <input type="search" aria-label="Search available tools" placeholder="Find a tool or connector…" value={search} onChange={event => setSearch(event.target.value)} />}
    <div className="connector-tool-groups">{items.map(item => {
      const visible = item.tools.filter(tool => `${item.id} ${tool.name} ${tool.description}`.toLowerCase().includes(search.toLowerCase()));
      if (!visible.length) return null;
      const count = item.tools.filter(tool => isSelected(item.id, tool.name)).length;
      return <details key={item.id} className="connector-tool-group"><summary>{item.id} <small>{count}/{item.tools.length} selected</small></summary>
        <label><input type="checkbox" aria-label={item.id} checked={count === item.tools.length && count > 0} disabled={busy} onChange={event => toggleTools(item, item.tools.map(tool => tool.name), event.target.checked)} />All {item.id} tools</label>
        {visible.map(tool => <label key={tool.name} className="connector-tool-choice"><input type="checkbox" aria-label={tool.name} checked={isSelected(item.id, tool.name)} disabled={busy || (!isSelected(item.id, tool.name) && toolCount >= 32)} onChange={event => toggleTools(item, [tool.name], event.target.checked)} /><span><strong>{tool.name}</strong><small>{tool.description || 'No description provided.'}</small></span></label>)}
      </details>;
    })}</div>
    {selectedTools.some(tool => !items.some(item => item.id === tool.connectorId && item.tools.some(available => available.name === tool.toolName))) && <p role="status">Some selected tools are unavailable. <button className="secondary" disabled={busy} onClick={() => onToolsChange(selectedTools.filter(tool => items.some(item => item.id === tool.connectorId && item.tools.some(available => available.name === tool.toolName))))}>Remove unavailable tools</button></p>}
    {!items.length && <p>Connect a service in Connectors to make its tools available here.</p>}
    {error && <p role="alert">{error}</p>}
  </details></>;
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
