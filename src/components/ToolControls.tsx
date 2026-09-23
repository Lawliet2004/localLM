import { useEffect, useRef, useState } from 'react';
import { AlertCircle, Check, ChevronDown, Eye, Shield, ShieldCheck, X } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, ConnectorView, ToolApproval, ToolSelection } from '../lib/types';

const connectorName = (item: ConnectorView) => (item.authType === 'local' ? item.description : item.id);
const workspaceName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() || path;

interface ModeOption {
  key: AccessMode;
  label: string;
  badge: string;
  badgeClass: string;
  desc: string;
  Icon: typeof ShieldCheck;
}

const PERMISSION_OPTIONS: ModeOption[] = [
  {
    key: 'ask',
    label: 'Ask for approval',
    badge: 'Safe',
    badgeClass: 'badge-safe',
    desc: 'Every tool action asks first. Review and approve before any tool runs.',
    Icon: ShieldCheck,
  },
  {
    key: 'autoApprove',
    label: 'Auto-approve reads',
    badge: 'Reads only',
    badgeClass: 'badge-info',
    desc: 'Workspace reads run automatically. Writes, code, and connectors ask.',
    Icon: Eye,
  },
  {
    key: 'fullAccess',
    label: 'Full access',
    badge: 'Unrestricted',
    badgeClass: 'badge-warning',
    desc: 'Selected tools run without prompts, including code and external changes.',
    Icon: AlertCircle,
  },
];

export function PermissionSelector({
  accessMode = 'ask',
  onAccessModeChange,
  busy = false,
  nativeAvailable: isNativeAvailable = true,
  onOpen,
}: {
  accessMode?: AccessMode;
  onAccessModeChange?: (mode: AccessMode) => void;
  busy?: boolean;
  nativeAvailable?: boolean;
  onOpen?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    function dismiss(event: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, []);

  const currentOption = PERMISSION_OPTIONS.find(opt => opt.key === accessMode) || PERMISSION_OPTIONS[0];
  const CurrentIcon = currentOption.Icon;
  const disabled = !isNativeAvailable || busy || !onAccessModeChange;

  function handleToggle() {
    if (disabled) return;
    setOpen(prev => {
      const next = !prev;
      if (next) onOpen?.();
      return next;
    });
  }

  function handleSelect(mode: AccessMode) {
    onAccessModeChange?.(mode);
    setOpen(false);
    triggerRef.current?.focus();
  }

  return (
    <div
      ref={containerRef}
      className={`permission-bar permission-menu-container permission-${accessMode}`}
      onKeyDown={event => {
        if (event.key === 'Escape' && open) {
          event.stopPropagation();
          setOpen(false);
          triggerRef.current?.focus();
        }
      }}
    >
      <label className="sr-only">
        Permissions
        <select
          aria-label="Permission mode"
          value={accessMode}
          disabled={disabled}
          onChange={event => onAccessModeChange?.(event.target.value as AccessMode)}
          tabIndex={-1}
        >
          <option value="ask">Ask for approval</option>
          <option value="autoApprove">Auto-approve reads</option>
          <option value="fullAccess">Full access</option>
        </select>
      </label>
      <span className="sr-only" aria-live="polite">
        {accessMode === 'fullAccess'
          ? 'Selected tools run without prompts, including code and external changes.'
          : accessMode === 'autoApprove'
          ? 'Workspace reads run automatically. Other actions ask.'
          : 'Every tool action asks first.'}
      </span>

      <button
        ref={triggerRef}
        type="button"
        className={`permission-trigger ${open ? 'active' : ''} ${accessMode === 'fullAccess' ? 'permission-trigger-full' : ''}`}
        aria-label={`Permission mode: ${currentOption.label}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        onClick={handleToggle}
        title={`Permission mode: ${currentOption.label}`}
      >
        <CurrentIcon size={14} className={`permission-mode-icon icon-${accessMode}`} />
        <span className="permission-trigger-label">{currentOption.label}</span>
        <ChevronDown size={13} className={`permission-chevron ${open ? 'open' : ''}`} />
      </button>

      {open && (
        <div
          className="permission-popover"
          role="dialog"
          aria-label="Select permission mode"
        >
          <div className="permission-popover-header">
            <div className="permission-popover-title">
              <Shield size={13} />
              <span>Permission Mode</span>
            </div>
            <button
              type="button"
              className="icon-button permission-close"
              aria-label="Close permission menu"
              onClick={() => {
                setOpen(false);
                triggerRef.current?.focus();
              }}
            >
              <X size={13} />
            </button>
          </div>

          <div className="permission-popover-options" aria-label="Permission options">
            {PERMISSION_OPTIONS.map(opt => {
              const isSelected = opt.key === accessMode;
              const OptIcon = opt.Icon;
              return (
                <button
                  key={opt.key}
                  type="button"
                  aria-pressed={isSelected}
                  className={`permission-option-card ${isSelected ? 'selected' : ''} ${opt.key === 'fullAccess' ? 'option-full-access' : ''}`}
                  onClick={() => handleSelect(opt.key)}
                >
                  <div className="permission-option-header">
                    <div className="permission-option-icon-label">
                      <div className={`permission-option-icon-wrap icon-${opt.key}`}>
                        <OptIcon size={15} />
                      </div>
                      <strong>{opt.label}</strong>
                    </div>
                    <div className="permission-option-badges">
                      <span className={`permission-badge ${opt.badgeClass}`}>{opt.badge}</span>
                      {isSelected && <Check size={14} className="permission-check-icon" />}
                    </div>
                  </div>
                  <p className="permission-option-desc">{opt.desc}</p>
                </button>
              );
            })}
          </div>

          <div className="permission-popover-footer">
            <span className="permission-footer-hint">
              {accessMode === 'fullAccess'
                ? 'Selected tools run without prompts, including code and external changes.'
                : accessMode === 'autoApprove'
                ? 'Workspace reads run automatically. Other actions ask.'
                : 'Every tool action asks first.'}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

interface ToolsSettingsProps {
  selected: string[]; onChange: (ids: string[]) => void;
  selectedTools?: ToolSelection[]; onToolsChange?: (tools: ToolSelection[]) => void;
  accessMode?: AccessMode; onAccessModeChange?: (mode: AccessMode) => void;
  preset?: string; onPresetChange?: (preset: string) => void;
  busy: boolean; loading?: boolean; saving?: boolean;
  scopeLabel?: string; onBack?: () => void;
  onNavigate?: (page: 'connectors' | 'execution' | 'skills') => void;
}

export function ToolsSettings({ scopeLabel, onBack, selected, onChange, selectedTools = [], onToolsChange, accessMode = 'ask', onAccessModeChange, preset = 'standard', onPresetChange, busy, loading = false, saving = false, onNavigate }: ToolsSettingsProps) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [hasDaytona, setHasDaytona] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(true);
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
  const connectedItems = items.filter(item => item.connected);
  const toolCount = selectedTools.length + (selected.includes('__workspace') ? 5 : 0) + (selected.includes('__execution') ? 1 : 0) + (selected.includes('__daytona') ? 1 : 0) + (activeSkills.length ? 1 : 0);
  const workspaceMissing = !pending && !workspace;
  const daytonaMissing = !pending && !hasDaytona;
  const toolUnavailable = (tool: ToolSelection) => !pending && !connectedItems.some(item => item.id === tool.connectorId && item.tools.some(available => available.name === tool.toolName));
  const unavailableCount = (selected.includes('__workspace') && workspaceMissing ? 1 : 0)
    + (selected.includes('__execution') && workspaceMissing ? 1 : 0)
    + (selected.includes('__daytona') && daytonaMissing ? 1 : 0)
    + selectedTools.filter(toolUnavailable).length;
  const summary = loading ? 'Tools · Loading…' : toolCount ? `Tools · ${toolCount} selected` : 'Tools · Off';
  const summaryDetails = `${unavailableCount ? ` · ${unavailableCount} unavailable` : ''}${activeSkills.length ? ` · ${activeSkills.length} active skills` : ''}${saving ? ' · Saving…' : ''}`;
  async function chooseWorkspace() {
    setChoosing(true); setError('');
    try { const path = await open({ directory: true, multiple: false, title: 'Choose workspace folder' }); if (typeof path === 'string') { await api.setWorkspace(path); setWorkspace(path); } }
    catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(false); }
  }
  return <div className="settings-page tools-settings">
    <div className="page-heading"><p className="eyebrow">ASSISTANT ACCESS</p><h1>Tools</h1><p>Choose what your assistant can use to work with files and run code.</p></div>
    <div className="tools-scope"><div><strong>{scopeLabel ? `Editing tools for: ${scopeLabel}` : 'Tools for your next conversation'}</strong><p>Changes save automatically and become the defaults for new chats. Other existing chats keep their own choices.</p></div>{onBack && <button type="button" className="secondary" onClick={onBack}>Back to chat</button>}</div>
    <p className="tools-summary" aria-live="polite">{summary}{summaryDetails}</p>
    {unavailableCount > 0 && <p className="tool-unavailable">Remove unavailable selections below or complete their setup before sending a message.</p>}
    <div className="tool-group"><h2>Workspace</h2><p>Choose a folder, then enable file access so your assistant can read and edit files there.</p>
      <label><input type="checkbox" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || pending || (!selected.includes('__workspace') && (!workspace || toolCount + 5 > 32))} onChange={event => onChange(event.target.checked ? [...selected, '__workspace'] : selected.filter(id => id !== '__workspace'))} />Workspace files</label>
      <button type="button" className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{workspace ? 'Change folder' : 'Choose folder'}</button>
      {workspace ? <span className="workspace-choice"><strong>{workspaceName(workspace)}</strong><details className="workspace-path"><summary>Full path</summary><code>{workspace}</code></details></span> : <small>{pending ? 'Checking for a workspace folder…' : 'Choose a folder to let the assistant read and edit files there.'}</small>}
    </div>
    <div className="tool-group"><h2>Code execution</h2><p>Local code runs unsandboxed, with this Windows account’s files and network. Cloud execution cannot be started.</p>
      <label><input type="checkbox" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || pending || (!selected.includes('__execution') && (!workspace || toolCount >= 32))} onChange={event => onChange(event.target.checked ? [...selected, '__execution'] : selected.filter(id => id !== '__execution'))} />Local code <small>Unsandboxed · {accessMode === 'fullAccess' ? 'runs without prompts' : 'approval required'}</small></label>
      {selected.includes('__execution') && workspaceMissing && <p className="tool-unavailable" role="status">Selected but unavailable: choose a workspace folder above.</p>}
      <label><input type="checkbox" aria-label="Daytona cloud code" checked={selected.includes('__daytona')} disabled={busy || pending || !selected.includes('__daytona')} onChange={event => { if (!event.target.checked) onChange(selected.filter(id => id !== '__daytona')); }} />Daytona cloud code <small>Unavailable. A saved key is left in place. This does not start a sandbox.</small></label>
      {selected.includes('__daytona') && <p className="tool-unavailable">This chat still has cloud execution selected. Uncheck it. Historical records stay readable, and no new sandbox is created.</p>}
    </div>
    <div className="tool-group">
      <h2>Tool profile preset</h2>
      <p>Select an explicit profile to control offered harness tools without changing custom connector choices.</p>
      <div className="preset-selector" role="radiogroup" aria-label="Tool preset">
        {[
          { id: 'standard', name: 'Standard', desc: 'Lean local toolset for general assistance' },
          { id: 'chat', name: 'Chat', desc: 'Conversational assistant with minimal tool surface' },
          { id: 'research', name: 'Research', desc: 'Live search and fetch tools' },
          { id: 'coding', name: 'Coding', desc: 'Code editing, execution, and programmatic tools' },
        ].map(p => (
          <label key={p.id} className={`preset-option ${preset === p.id ? 'active' : ''}`}>
            <input
              type="radio"
              name="tool-preset"
              value={p.id}
              checked={preset === p.id}
              disabled={busy || !onPresetChange}
              onChange={() => onPresetChange?.(p.id)}
            />
            <span><strong>{p.name}</strong> · <small>{p.desc}</small></span>
          </label>
        ))}
      </div>
    </div>
    <section className="tool-group"><h2>Permissions</h2><p>Choose when the assistant should ask before using a tool. Public web read, workspace read, workspace write, local code, and shell are separate. Ask stays ask. The model cannot grant access. Offline mode turns off external retrieval and does not make shell or local MCP network-isolated.</p><PermissionSelector accessMode={accessMode} onAccessModeChange={onAccessModeChange} busy={busy} nativeAvailable={nativeAvailable} /></section>
    {selectedTools.length > 0 && <div className="tool-group"><h2>Selected connector tools</h2>
      {selectedTools.map(tool => <div key={`${tool.connectorId}:${tool.toolName}`}>
        <span>{tool.connectorId} · {tool.toolName}{toolUnavailable(tool) ? ' · Not connected' : ''}</span>
        <button type="button" className="secondary" aria-label={`Remove ${tool.connectorId} · ${tool.toolName}`} disabled={busy || !onToolsChange} onClick={() => onToolsChange?.(selectedTools.filter(item => item.connectorId !== tool.connectorId || item.toolName !== tool.toolName))}>Remove</button>
      </div>)}
      {onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('connectors')}>Open Connectors</button>}
    </div>}
    {activeSkills.length > 0 && <p>Active skills: {activeSkills.join(', ')}. One tool enables reading their package files. Deactivate skills in Skills to remove it.</p>}
    <section className="tool-group"><h2>Connected services</h2><p>{connectedItems.length ? `Connected: ${connectedItems.map(item => connectorName(item)).join(', ')}.` : 'No services connected yet.'} Add or reconnect services in Connectors.</p>{onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('connectors')}>Manage connectors</button>}</section>
    {error && <p role="alert">{error}</p>}
  </div>;
}

export function ApprovalDialog({ request, onResolve, onResolveAskUser }: { request: ToolApproval; onResolve: (allow: boolean) => Promise<void>; onResolveAskUser?: (choice: string | null) => Promise<void> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [freeform, setFreeform] = useState('');
  useEffect(() => { dialog.current?.showModal(); }, []);
  async function resolve(allow: boolean) {
    setBusy(true); setError('');
    try { await onResolve(allow); }
    catch (e) { setError(errorMessage(e)); setBusy(false); }
  }
  async function resolveChoice(choice: string | null) {
    if (!onResolveAskUser) { await resolve(choice !== null); return; }
    setBusy(true); setError('');
    try { await onResolveAskUser(choice); }
    catch (e) { setError(errorMessage(e)); setBusy(false); }
  }
  const isAskUser = request.kind === 'ask_user' || request.name === 'ask_user';
  const question = typeof request.arguments.question === 'string' ? request.arguments.question : '';
  const options = Array.isArray(request.arguments.options) ? (request.arguments.options as unknown[]).filter((o): o is string => typeof o === 'string') : [];
  if (isAskUser) {
    return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void resolveChoice(null); }}>
      <p className="eyebrow">QUESTION FROM ASSISTANT</p><h2 id="approval-title">{question || 'The assistant has a question'}</h2>
      {options.length > 0 && <div className="connector-actions" role="group" aria-label="Answer options">
        {options.map(option => <button key={option} type="button" className="secondary" disabled={busy} onClick={() => void resolveChoice(option)}>{option}</button>)}
      </div>}
      <form onSubmit={event => { event.preventDefault(); void resolveChoice(freeform.trim() ? freeform.trim() : (options[0] ?? 'yes')); }}>
        <label>Or type your own answer<input aria-label="Your answer" maxLength={4000} value={freeform} disabled={busy} onChange={e => setFreeform(e.target.value)} placeholder="Type an answer…" /></label>
        <div className="connector-actions">
          <button type="button" className="secondary" disabled={busy} onClick={() => void resolveChoice(null)}>Decline</button>
          <button type="submit" className="primary" disabled={busy}>Answer</button>
        </div>
      </form>
      {error && <p role="alert">{error}</p>}
    </dialog>;
  }
  return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void resolve(false); }}>
    <p className="eyebrow">ACTION REQUEST</p><h2 id="approval-title">{request.localServerName ? `Allow local server ${request.localServerName} to run this tool?` : request.connector === 'Local execution' ? 'Run this code on your computer?' : `Allow ${request.connector} to run this tool?`}</h2>
    {request.localServerName && <p>This local server runs with your account’s file and network permissions. Review the arguments before allowing this action. Server ID: <code>{request.connector}</code></p>}
    <p><strong>{request.name}</strong></p><p>{request.connector === 'Local execution' ? 'This code runs unsandboxed with your Windows account’s permissions. It can access files outside the workspace and the network. Review the complete code before allowing it.' : request.connector === 'Daytona' ? 'Cloud execution is unavailable. This call will not be sent. Historical cloud records stay readable.' : request.connector === 'Workspace' ? 'This action will access the workspace folder on your computer using the arguments below.' : request.connector === 'Skills' ? 'Read a verified file from an active skill’s local package. This does not execute scripts or contact a service.' : 'The arguments below will be sent to this service. The action may read or change external data.'}</p>
    {(request.connector === 'Local execution' || request.connector === 'Daytona') && typeof request.arguments.code === 'string' ? <><p>Language: {String(request.arguments.language)} · Timeout: {String(request.arguments.timeout_seconds ?? 60)} seconds</p><pre className="approval-code">{request.arguments.code}</pre><details><summary>Full request</summary><pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre></details></> : <pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre>}
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button type="button" autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button type="button" className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
