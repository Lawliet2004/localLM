import { useEffect, useRef, useState } from 'react';
import { AlertCircle, Check, ChevronDown, ChevronRight, Eye, Shield, ShieldCheck, X } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, ConnectorView, ToolApproval, ToolSelection } from '../lib/types';
import { MarkdownView } from './MarkdownView';

const connectorName = (item: ConnectorView) => (item.authType === 'local' ? item.description : item.id);
const workspaceName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() || path;
const WORKSPACE_TOOL_COUNT = 28;

interface ModeOption {
  key: AccessMode;
  label: string;
  shortLabel: string;
  badge: string;
  desc: string;
  Icon: typeof ShieldCheck;
}

const PERMISSION_OPTIONS: ModeOption[] = [
  {
    key: 'ask',
    label: 'Ask for approval',
    shortLabel: 'Ask',
    badge: 'Safe',
    desc: 'Every tool action asks first. Review and approve before any tool runs.',
    Icon: ShieldCheck,
  },
  {
    key: 'autoApprove',
    label: 'Auto-approve reads',
    shortLabel: 'Auto reads',
    badge: 'Reads only',
    desc: 'Workspace reads run automatically. Writes, code, and connectors ask.',
    Icon: Eye,
  },
  {
    key: 'fullAccess',
    label: 'Full access',
    shortLabel: 'Full access',
    badge: 'Unrestricted',
    desc: 'Selected tools run automatically unless their settings require approval.',
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
          ? 'Selected tools run automatically unless their settings require approval.'
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
            <div className="permission-popover-heading">
              <span className="permission-popover-mark" aria-hidden="true">
                <Shield size={14} />
              </span>
              <div className="permission-popover-copy">
                <span className="permission-popover-title">Permission mode</span>
                <span className="permission-popover-subtitle">Choose when tools need approval</span>
              </div>
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
              <X size={14} />
            </button>
          </div>

          <div className="permission-modes" role="group" aria-label="Permission options">
            {PERMISSION_OPTIONS.map(opt => {
              const isSelected = opt.key === accessMode;
              const OptIcon = opt.Icon;
              return (
                <button
                  key={opt.key}
                  type="button"
                  aria-label={opt.label}
                  aria-pressed={isSelected}
                  title={opt.desc}
                  className={`permission-mode-card option-${opt.key} ${isSelected ? 'selected' : ''}`}
                  onClick={() => handleSelect(opt.key)}
                >
                  <span className="permission-mode-card-top">
                    <span className="permission-mode-card-icon" aria-hidden="true">
                      <OptIcon size={17} />
                    </span>
                    {isSelected && (
                      <span className="permission-mode-card-check" aria-hidden="true">
                        <Check size={11} />
                      </span>
                    )}
                  </span>
                  <span className="permission-mode-card-label">{opt.shortLabel}</span>
                  <span className="permission-mode-card-badge">{opt.badge}</span>
                </button>
              );
            })}
          </div>

          <div className={`permission-mode-detail option-${accessMode}`}>
            <CurrentIcon size={14} className="permission-mode-detail-icon" aria-hidden="true" />
            <span className="permission-mode-detail-copy">
              <strong>{currentOption.label}</strong>
              <span>{currentOption.desc}</span>
            </span>
          </div>

          <div className="permission-popover-footer">
            <ShieldCheck size={13} aria-hidden="true" />
            <span>The model cannot grant itself access.</span>
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
  onNavigate?: (page: 'connectors' | 'execution' | 'skills' | 'browser-use' | 'computer-use') => void;
  onChooseWorkspace?: (path: string) => Promise<string>;
}

export function ToolsSettings({ scopeLabel, onBack, selected, onChange, selectedTools = [], onToolsChange, accessMode = 'ask', onAccessModeChange, preset = 'standard', onPresetChange, busy, loading = false, saving = false, onNavigate, onChooseWorkspace }: ToolsSettingsProps) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [hasDaytona, setHasDaytona] = useState(false);
  const [automation, setAutomation] = useState({ browserEnabled: false, computerEnabled: false });
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(true);
  useEffect(() => {
    if (!nativeAvailable) { setPending(false); return; }
    let disposed = false;
    let remaining = 4;
    const settle = () => { remaining -= 1; if (remaining === 0 && !disposed) setPending(false); };
    const fail = (e: unknown) => { if (!disposed) setError(current => current || errorMessage(e)); };
    if (api.getAutomationConfig) api.getAutomationConfig().then(value => { if (!disposed) setAutomation(value); }).catch(fail);
    api.hasDaytonaKey().then(value => { if (!disposed) setHasDaytona(value); }).catch(fail).finally(settle);
    api.listConnectors().then(value => { if (!disposed) setItems(value); }).catch(fail).finally(settle);
    api.listSkills().then(value => { if (!disposed) setActiveSkills(value.filter(item => item.active).map(item => item.id)); }).catch(fail).finally(settle);
    api.getWorkspace().then(value => { if (!disposed) setWorkspace(value.path); }).catch(fail).finally(settle);
    return () => { disposed = true; };
  }, []);
  const connectedItems = items.filter(item => item.connected);
  const toolCount = (selected.includes('__browser') ? 11 : 0) + (selected.includes('__computer') ? 7 : 0) + selectedTools.length + (selected.includes('__workspace') ? WORKSPACE_TOOL_COUNT : 0) + (selected.includes('__execution') ? 1 : 0) + (selected.includes('__daytona') ? 1 : 0) + (activeSkills.length ? 1 : 0);
  const workspaceMissing = !pending && !workspace;
  const daytonaMissing = !pending && !hasDaytona;
  const toolUnavailable = (tool: ToolSelection) => !pending && !connectedItems.some(item => item.id === tool.connectorId && item.tools.some(available => available.name === tool.toolName));
  const unavailableCount = (selected.includes('__workspace') && workspaceMissing ? 1 : 0)
    + (selected.includes('__execution') && workspaceMissing ? 1 : 0)
    + (selected.includes('__daytona') && daytonaMissing ? 1 : 0)
    + (selected.includes('__browser') && !automation.browserEnabled ? 1 : 0)
    + (selected.includes('__computer') && !automation.computerEnabled ? 1 : 0)
    + selectedTools.filter(toolUnavailable).length;
  const selectionSummary = [
    selected.length ? `${selected.length} ${selected.length === 1 ? 'source' : 'sources'}` : '',
    selectedTools.length ? `${selectedTools.length} connector ${selectedTools.length === 1 ? 'tool' : 'tools'}` : '',
  ].filter(Boolean).join(' · ');
  const summary = loading ? 'Tools · Loading…' : selectionSummary ? `Tools · ${selectionSummary}` : 'Tools · No sources selected';
  const summaryDetails = `${unavailableCount ? ` · ${unavailableCount} unavailable` : ''}${activeSkills.length ? ` · ${activeSkills.length} active skills` : ''}${saving ? ' · Saving…' : ''}`;
  async function chooseWorkspace() {
    setChoosing(true); setError('');
    try { const path = await open({ directory: true, multiple: false, title: 'Choose workspace folder' }); if (typeof path === 'string') { const nextPath = onChooseWorkspace ? await onChooseWorkspace(path) : (await api.setWorkspace(path), path); setWorkspace(nextPath); } }
    catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(false); }
  }
  return <div className="settings-page tools-settings">
    <div className="page-heading"><h1>Tools</h1><p>Choose what your assistant can use to work with files and run code.</p></div>
    <div className="tools-scope"><div><strong>{scopeLabel ? `Editing tools for: ${scopeLabel}` : 'Tools for your next conversation'}</strong><p>Changes save automatically and become the defaults for new chats. Other existing chats keep their own choices.</p></div>{onBack && <button type="button" className="secondary" onClick={onBack}>Back to chat</button>}</div>
    <p className="tools-summary" aria-live="polite">{summary}{summaryDetails}</p><p className="preference-footnote">{toolCount} selected source tools. Preset and clock tools also count toward the 64-tool limit. The composer checks the assembled catalog before sending.</p>
    {unavailableCount > 0 && <p className="tool-unavailable">Remove unavailable selections below or complete their setup before sending a message.</p>}
    <section className="preference-section" aria-labelledby="tools-workspace-heading">
      <div className="preference-section-heading"><h2 id="tools-workspace-heading">Workspace</h2><p>Choose a folder, then enable file access.</p></div>
      <div className="preference-group">
        <div className="preference-row preference-toggle">
          <div className="preference-copy"><span className="preference-title">Workspace files</span><span className="preference-desc">Read, search, and edit files in the workspace folder.</span></div>
          <input type="checkbox" className="switch" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || pending || (!selected.includes('__workspace') && (!workspace || toolCount + WORKSPACE_TOOL_COUNT > 64))} onChange={event => onChange(event.target.checked ? [...selected, '__workspace'] : selected.filter(id => id !== '__workspace'))} />
        </div>
        <div className="preference-row">
          <div className="preference-copy"><span className="preference-title">Folder</span>
            {workspace ? <div className="workspace-choice"><strong>{workspaceName(workspace)}</strong><details className="workspace-path"><summary>Full path</summary><code>{workspace}</code></details></div>
              : <span className="preference-desc">{pending ? 'Checking for a workspace folder…' : 'Choose a folder to let the assistant read and edit files there.'}</span>}
          </div>
          <button type="button" className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{choosing ? 'Choosing…' : workspace ? 'Change folder' : 'Choose folder'}</button>
        </div>
      </div>
    </section>
    <section className="preference-section" aria-labelledby="tools-execution-heading">
      <div className="preference-section-heading"><h2 id="tools-execution-heading">Code execution</h2><p>Local code runs unsandboxed, with this account’s files and network.</p></div>
      <div className="preference-group">
        <div className="preference-row preference-toggle">
          <div className="preference-copy"><span className="preference-title">Local code</span><span className="preference-desc">Unsandboxed · {accessMode === 'fullAccess' ? 'follows tool approval settings' : 'approval required'}</span></div>
          <input type="checkbox" className="switch" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || pending || (!selected.includes('__execution') && (!workspace || toolCount >= 64))} onChange={event => onChange(event.target.checked ? [...selected, '__execution'] : selected.filter(id => id !== '__execution'))} />
        </div>
        {selected.includes('__execution') && workspaceMissing && <div className="settings-note"><p className="tool-unavailable" role="status">Selected but unavailable: choose a workspace folder above.</p></div>}
        <div className="preference-row preference-toggle">
          <div className="preference-copy"><span className="preference-title">Daytona cloud code</span><span className="preference-desc">Unavailable. A saved key is left in place; this does not start a sandbox.</span></div>
          <input type="checkbox" className="switch" aria-label="Daytona cloud code" checked={selected.includes('__daytona')} disabled={busy || pending || !selected.includes('__daytona')} onChange={event => { if (!event.target.checked) onChange(selected.filter(id => id !== '__daytona')); }} />
        </div>
        {selected.includes('__daytona') && <div className="settings-note"><p className="tool-unavailable">This chat still has cloud execution selected. Uncheck it. Historical records stay readable, and no new sandbox is created.</p></div>}
      </div>
    </section>
    <section className="preference-section" aria-labelledby="tools-automation-heading">
      <div className="preference-section-heading"><h2 id="tools-automation-heading">Browser &amp; computer</h2><p>Automation actions and screenshots ask for approval unless Full access is selected.</p></div>
      <div className="preference-group">
        {([['__browser', 'Browser use', automation.browserEnabled, 11], ['__computer', 'Computer use', automation.computerEnabled, 7]] as const).map(([id, label, enabled, count]) => <div className="preference-row preference-toggle" key={id}>
          <div className="preference-copy"><span className="preference-title">{label}</span><span className="preference-desc">{enabled ? `${count} tools` : `Set up in ${label} settings`}</span></div>
          <input type="checkbox" className="switch" aria-label={label} checked={selected.includes(id)} disabled={busy || pending || (!selected.includes(id) && (!enabled || ['chat', 'minimal'].includes(preset) || toolCount + count > 64))} onChange={event => onChange(event.target.checked ? [...selected, id] : selected.filter(source => source !== id))} />
        </div>)}
        <button type="button" className="preference-row preference-nav" aria-labelledby="tools-browser-nav" onClick={() => onNavigate?.('browser-use')} disabled={busy}>
          <span className="preference-copy"><span className="preference-title" id="tools-browser-nav">Configure browser use</span></span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>
        <button type="button" className="preference-row preference-nav" aria-labelledby="tools-computer-nav" onClick={() => onNavigate?.('computer-use')} disabled={busy}>
          <span className="preference-copy"><span className="preference-title" id="tools-computer-nav">Configure computer use</span></span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>
        {['chat', 'minimal'].includes(preset) && <div className="settings-note"><p>Choose Standard, Research, or Coding to use automation.</p></div>}
      </div>
    </section>
    <section className="preference-section" aria-labelledby="tools-preset-heading">
      <div className="preference-section-heading"><h2 id="tools-preset-heading">Tool profile preset</h2><p>Controls which harness tools are offered, without changing connector choices.</p></div>
      <div className="preference-group" role="radiogroup" aria-label="Tool preset">
        {[
          { id: 'standard', name: 'Standard', desc: 'Lean local toolset for general assistance' },
          { id: 'chat', name: 'Chat', desc: 'Conversational assistant with minimal tool surface' },
          { id: 'research', name: 'Research', desc: 'Live search and fetch tools' },
          { id: 'coding', name: 'Coding', desc: 'Code editing, execution, and programmatic tools' },
          { id: 'code', name: 'Code (PTC)', desc: 'Group approved tool calls in a bounded batch' },
          { id: 'minimal', name: 'Minimal', desc: 'Persistent shell and file editing for benchmarking' },
          { id: 'creator', name: 'Creator', desc: 'Workflows, subagents, and runtime authoring tools' },
        ].map(p => (
          <label key={p.id} className="preference-row preference-radio">
            <input
              type="radio"
              name="tool-preset"
              value={p.id}
              checked={preset === p.id}
              disabled={busy || !onPresetChange}
              onChange={() => onPresetChange?.(p.id)}
            />
            <span className="preference-copy"><span className="preference-title">{p.name}</span><span className="preference-desc">{p.desc}</span></span>
          </label>
        ))}
      </div>
    </section>
    <section className="preference-section" aria-labelledby="tools-permissions-heading">
      <div className="preference-section-heading"><h2 id="tools-permissions-heading">Permissions</h2><p>When the assistant should ask before using a tool.</p></div>
      <div className="preference-group overflow-visible">
        <div className="preference-row">
          <div className="preference-copy"><span className="preference-title">Permission mode</span><span className="preference-desc">Ask stays ask — the model cannot grant access.</span></div>
          <PermissionSelector accessMode={accessMode} onAccessModeChange={onAccessModeChange} busy={busy} nativeAvailable={nativeAvailable} />
        </div>
        <div className="settings-note"><p>Public web read, workspace read/write, local code, and shell are separate. Offline mode turns off external retrieval without making shell or local MCP network-isolated.</p></div>
      </div>
    </section>
    {selectedTools.length > 0 && <section className="preference-section" aria-labelledby="tools-connectors-heading"><div className="preference-section-heading"><h2 id="tools-connectors-heading">Selected connector tools</h2></div>
      <div className="preference-group">
        {selectedTools.map(tool => <div className="preference-row" key={`${tool.connectorId}:${tool.toolName}`}>
          <div className="preference-copy"><span className="preference-title">{tool.connectorId} · {tool.toolName}{toolUnavailable(tool) ? ' · Not connected' : ''}</span></div>
          <button type="button" className="secondary" aria-label={`Remove ${tool.connectorId} · ${tool.toolName}`} disabled={busy || !onToolsChange} onClick={() => onToolsChange?.(selectedTools.filter(item => item.connectorId !== tool.connectorId || item.toolName !== tool.toolName))}>Remove</button>
        </div>)}
        {onNavigate && <button type="button" className="preference-row preference-nav" aria-labelledby="tools-open-connectors" onClick={() => onNavigate('connectors')}>
          <span className="preference-copy"><span className="preference-title" id="tools-open-connectors">Open Connectors</span></span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>}
      </div>
    </section>}
    {activeSkills.length > 0 && <p className="preference-footnote">Active skills: {activeSkills.join(', ')}. One tool enables reading their package files. Deactivate skills in Skills to remove it.</p>}
    <section className="preference-section" aria-labelledby="tools-services-heading"><div className="preference-section-heading"><h2 id="tools-services-heading">Connected services</h2></div>
      <div className="preference-group">
        <div className="settings-note"><p>{connectedItems.length ? `Connected: ${connectedItems.map(item => connectorName(item)).join(', ')}.` : 'No services connected yet.'} Add or reconnect services in Connectors.</p></div>
        {onNavigate && <button type="button" className="preference-row preference-nav" aria-labelledby="tools-manage-connectors" onClick={() => onNavigate('connectors')}>
          <span className="preference-copy"><span className="preference-title" id="tools-manage-connectors">Manage connectors</span></span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>}
      </div>
    </section>
    {error && <p role="alert">{error}</p>}
  </div>;
}

type McpFieldValue = string | boolean | string[] | undefined;

/// Native structured form for one MCP input request (form elicitation or an
/// `inputRequired` continuation). Server labels/descriptions render as plain
/// text; nothing here navigates, executes, or pre-submits a default — Send,
/// Decline and Cancel are explicit user actions. Invalid answers surface the
/// backend's rejection while the request stays pending.
function McpInputDialog({ request, onResolve }: { request: ToolApproval; onResolve: (action: 'accept' | 'decline' | 'cancel', content?: Record<string, unknown>) => Promise<void> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const form = request.form;
  const fields = form?.fields ?? [];
  const [values, setValues] = useState<Record<string, McpFieldValue>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => { dialog.current?.showModal(); }, []);
  const label = (field: import('../lib/types').McpInputField) => field.title || field.name;
  const set = (name: string, value: McpFieldValue) => setValues(current => ({ ...current, [name]: value }));
  async function decide(action: 'accept' | 'decline' | 'cancel', content?: Record<string, unknown>) {
    setBusy(true); setError('');
    try { await onResolve(action, content); }
    catch (e) { setError(errorMessage(e)); setBusy(false); }
  }
  function accept() {
    const content: Record<string, unknown> = {};
    for (const field of fields) {
      const raw = values[field.name];
      const empty = raw === undefined || raw === '' || (Array.isArray(raw) && raw.length === 0);
      if (empty) {
        if (field.required) { setError(`${label(field)} is required.`); return; }
        continue;
      }
      switch (field.kind) {
        case 'text': {
          const text = String(raw);
          if (field.minLength !== undefined && text.length < field.minLength) { setError(`${label(field)} needs at least ${field.minLength} characters.`); return; }
          if (field.maxLength !== undefined && text.length > field.maxLength) { setError(`${label(field)} allows at most ${field.maxLength} characters.`); return; }
          content[field.name] = text;
          break;
        }
        case 'number':
        case 'integer': {
          const parsed = field.kind === 'integer' ? Number(raw) : Number.parseFloat(String(raw));
          if (!Number.isFinite(parsed) || (field.kind === 'integer' && !Number.isInteger(parsed))) { setError(`${label(field)} must be a ${field.kind === 'integer' ? 'whole number' : 'number'}.`); return; }
          if (field.minimum !== undefined && parsed < field.minimum) { setError(`${label(field)} must be at least ${field.minimum}.`); return; }
          if (field.maximum !== undefined && parsed > field.maximum) { setError(`${label(field)} must be at most ${field.maximum}.`); return; }
          content[field.name] = parsed;
          break;
        }
        case 'boolean': {
          if (typeof raw !== 'boolean') { setError(`${label(field)} needs a yes or no answer.`); return; }
          content[field.name] = raw;
          break;
        }
        case 'select': {
          if (!field.options?.some(option => option.value === raw)) { setError(`Choose an option for ${label(field)}.`); return; }
          content[field.name] = raw;
          break;
        }
        case 'multi': {
          const picked = Array.isArray(raw) ? raw : [];
          if (field.minItems !== undefined && picked.length < field.minItems) { setError(`Choose at least ${field.minItems} for ${label(field)}.`); return; }
          if (field.maxItems !== undefined && picked.length > field.maxItems) { setError(`Choose at most ${field.maxItems} for ${label(field)}.`); return; }
          if (picked.some(value => !field.options?.some(option => option.value === value))) { setError(`Invalid choice in ${label(field)}.`); return; }
          content[field.name] = picked;
          break;
        }
      }
    }
    void decide('accept', content);
  }
  const inputType = (format?: string) => format === 'email' ? 'email' : format === 'uri' ? 'url' : format === 'date' ? 'date' : format === 'date-time' ? 'datetime-local' : 'text';
  const source = request.localServerName || request.connector;
  return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void decide('cancel'); }}>
    <h2 id="approval-title">{source} is asking for input</h2>
    <p>The MCP server {request.localServerName ? <><code>{request.localServerName}</code> (id: <code>{request.connector}</code>)</> : <code>{request.connector}</code>} needs this information to continue <strong>{request.name}</strong>. Your answers are sent only to that server.</p>
    {form?.message && <p className="approval-purpose">{form.message}</p>}
    <form onSubmit={event => { event.preventDefault(); accept(); }}>
      {fields.map(field => <div key={field.name} className="mcp-form-field">
        {field.kind === 'boolean' ? (
          <label>{label(field)}{field.required ? ' *' : ''}
            <select aria-label={label(field)} value={values[field.name] === true ? 'yes' : values[field.name] === false ? 'no' : ''} disabled={busy}
              onChange={event => set(field.name, event.target.value === '' ? undefined : event.target.value === 'yes')}>
              <option value="">{field.required ? 'Choose…' : 'Unanswered'}{typeof field.default === 'boolean' ? ` (default: ${field.default ? 'yes' : 'no'})` : ''}</option>
              <option value="yes">Yes</option>
              <option value="no">No</option>
            </select>
          </label>
        ) : field.kind === 'select' ? (
          <label>{label(field)}{field.required ? ' *' : ''}
            <select aria-label={label(field)} value={typeof values[field.name] === 'string' ? values[field.name] as string : ''} disabled={busy} onChange={event => set(field.name, event.target.value === '' ? undefined : event.target.value)}>
              <option value="">Choose…{typeof field.default === 'string' ? ` (default: ${field.default})` : ''}</option>
              {field.options?.map(option => <option key={option.value} value={option.value}>{option.label || option.value}</option>)}
            </select>
          </label>
        ) : field.kind === 'multi' ? (
          <fieldset>
            <legend>{label(field)}{field.required ? ' *' : ''}</legend>
            {field.options?.map(option => {
              const picked = Array.isArray(values[field.name]) ? values[field.name] as string[] : [];
              return <label key={option.value} className="mcp-form-choice"><input type="checkbox" disabled={busy} checked={picked.includes(option.value)}
                onChange={event => set(field.name, event.target.checked ? [...picked, option.value] : picked.filter(value => value !== option.value))} />{option.label || option.value}</label>;
            })}
            {(field.minItems !== undefined || field.maxItems !== undefined) && <span className="preference-desc">{field.minItems !== undefined ? `at least ${field.minItems}` : ''}{field.minItems !== undefined && field.maxItems !== undefined ? ' · ' : ''}{field.maxItems !== undefined ? `at most ${field.maxItems}` : ''}</span>}
          </fieldset>
        ) : (
          <label>{label(field)}{field.required ? ' *' : ''}
            <input aria-label={label(field)} type={field.kind === 'text' ? inputType(field.format) : 'number'} step={field.kind === 'integer' ? 1 : field.kind === 'number' ? 'any' : undefined} disabled={busy}
              placeholder={field.default !== undefined ? `default: ${String(field.default)}` : undefined}
              value={typeof values[field.name] === 'string' ? values[field.name] as string : ''}
              onChange={event => set(field.name, event.target.value)} />
          </label>
        )}
        {field.description && <p className="preference-desc">{field.description}</p>}
      </div>)}
      <div className="connector-actions">
        <button type="button" className="secondary" disabled={busy} onClick={() => void decide('cancel')}>Cancel</button>
        <button type="button" className="secondary" disabled={busy} onClick={() => void decide('decline')}>Decline</button>
        <button type="submit" className="primary" disabled={busy}>Send</button>
      </div>
    </form>
    {error && <p role="alert">{error}</p>}
  </dialog>;
}

export function ApprovalDialog({ request, onResolve, onResolveAskUser, onResolveMcpInput }: { request: ToolApproval; onResolve: (allow: boolean) => Promise<void>; onResolveAskUser?: (choice: string | null) => Promise<void>; onResolveMcpInput?: (action: 'accept' | 'decline' | 'cancel', content?: Record<string, unknown>) => Promise<void> }) {
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
  if (request.kind === 'mcp_input' && request.form && onResolveMcpInput) {
    return <McpInputDialog request={request} onResolve={onResolveMcpInput} />;
  }
  const isAskUser = request.kind === 'ask_user';
  const isPlanApproval = request.kind === 'plan_approval';
  const question = typeof request.arguments.question === 'string' ? request.arguments.question : '';
  const options = Array.isArray(request.arguments.options) ? (request.arguments.options as unknown[]).filter((o): o is string => typeof o === 'string') : [];
  const sourceSnapshot = request.arguments.approved_source;
  const sourceCode = typeof request.arguments.code === 'string' ? request.arguments.code
    : sourceSnapshot && typeof sourceSnapshot === 'object' && 'code' in sourceSnapshot && typeof sourceSnapshot.code === 'string' ? sourceSnapshot.code : null;
  if (isPlanApproval) {
    const plan = typeof request.arguments.plan === 'string' ? request.arguments.plan : '';
    const summary = typeof request.arguments.summary === 'string' ? request.arguments.summary : '';
    return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void resolveChoice(null); }}>
      <h2 id="approval-title">Review the proposed plan</h2>
      {summary && <p className="approval-purpose">{summary}</p>}
      <div className="approval-plan"><MarkdownView content={plan} /></div>
      <form onSubmit={event => { event.preventDefault(); const feedback = freeform.trim(); if (!feedback) { setError('Describe the changes you want, or approve the plan.'); return; } void resolveChoice(feedback); }}>
        <label>Request changes<textarea aria-label="Plan feedback" maxLength={4000} value={freeform} disabled={busy} onChange={e => setFreeform(e.target.value)} placeholder="What should change?" /></label>
        <div className="connector-actions">
          <button type="button" className="secondary" disabled={busy} onClick={() => void resolveChoice(null)}>Decline</button>
          <button type="submit" className="secondary" disabled={busy}>Request changes</button>
          <button type="button" className="primary" disabled={busy} onClick={() => void resolveChoice('approved')}>Approve</button>
        </div>
      </form>
      {error && <p role="alert">{error}</p>}
    </dialog>;
  }
  if (isAskUser) {
    return <dialog ref={dialog} className="tool-approval" aria-labelledby="approval-title" onCancel={event => { event.preventDefault(); if (!busy) void resolveChoice(null); }}>
      <h2 id="approval-title">{question || 'The assistant has a question'}</h2>
      {options.length > 0 && <div className="connector-actions" role="group" aria-label="Answer options">
        {options.map(option => <button key={option} type="button" className="secondary" disabled={busy} onClick={() => void resolveChoice(option)}>{option}</button>)}
      </div>}
      <form onSubmit={event => { event.preventDefault(); const answer = freeform.trim(); if (!answer) { setError('Choose an option or enter an answer.'); return; } void resolveChoice(answer); }}>
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
    <h2 id="approval-title">{request.localServerName ? `Allow local server ${request.localServerName} to run this tool?` : request.connector === 'Local execution' ? 'Run this code on your computer?' : `Allow ${request.connector} to run this tool?`}</h2>
    {request.localServerName && <p>This local server runs with your account’s file and network permissions. Review the arguments before allowing this action. Server ID: <code>{request.connector}</code></p>}
    {request.purpose && <p className="approval-purpose">Current task: {request.purpose}</p>}
    <p><strong>{request.name}</strong></p><p>{request.connector === 'Local execution' ? 'This code runs unsandboxed with your account’s permissions. It can access files outside the workspace and the network. Review the complete code before allowing it.' : request.connector === 'Daytona' ? 'Cloud execution is unavailable. This call will not be sent. Historical cloud records stay readable.' : request.connector === 'Computer' ? 'This action can read your screen or control the focused desktop app. Coordinates use the primary monitor. Review the arguments before allowing it.' : request.connector === 'Browser' ? 'This action controls this conversation’s isolated browser. It may read pages, enter text, submit forms, or capture a screenshot.' : request.connector === 'Workspace' ? 'This action will access the workspace folder on your computer using the arguments below.' : request.connector === 'Skills' ? 'Read a verified file from an active skill’s local package. This does not execute scripts or contact a service.' : 'The arguments below will be sent to this service. The action may read or change external data.'}</p>
    {sourceCode !== null ? <><p>Language: {String(request.arguments.language ?? 'python')} · Timeout: {request.arguments.timeout_seconds === undefined ? 'Configured limit' : `${String(request.arguments.timeout_seconds)} seconds`}</p>{typeof request.arguments.file === 'string' && <p>Script: {request.arguments.file}. The file must still match this source when execution begins.</p>}<pre className="approval-code">{sourceCode}</pre><details><summary>Full request</summary><pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre></details></> : <pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre>}
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button type="button" autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button type="button" className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
