import { useEffect, useRef, useState } from 'react';
import { AlertCircle, Check, ChevronDown, Eye, ShieldCheck } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode, ConnectorView, EffectiveTool, InteractionMode, Profile, ToolApproval, ToolSelection } from '../lib/types';
import { RadioCards, SettingRow, SettingsGroup, SettingsHeader } from './Settings';

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

export const PERMISSION_OPTIONS: ModeOption[] = [
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
        <ChevronDown size={12} className={`permission-chevron ${open ? 'open' : ''}`} />
      </button>

      {open && (
        <div
          className="permission-popover"
          role="dialog"
          aria-label="Select permission mode"
        >
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
                <OptIcon size={15} className={`permission-option-icon icon-${opt.key}`} aria-hidden="true" />
                <span className="permission-option-copy">
                  <strong>{opt.label}</strong>
                  <span className="permission-option-desc">{opt.desc}</span>
                </span>
                <span className="permission-option-check" aria-hidden="true">{isSelected && <Check size={14} className="permission-check-icon" />}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

interface ToolsSettingsProps {
  selected: string[]; onChange: (ids: string[]) => void;
  selectedTools?: ToolSelection[]; onToolsChange?: (tools: ToolSelection[]) => void;
  accessMode?: AccessMode; onAccessModeChange?: (mode: AccessMode) => void;
  interactionMode?: InteractionMode; onInteractionModeChange?: (mode: InteractionMode) => void;
  preset?: string; onPresetChange?: (preset: string) => void;
  busy: boolean; loading?: boolean; saving?: boolean;
  scopeLabel?: string; conversationId?: string | null;
  onNavigate?: (page: 'connectors' | 'execution' | 'skills' | 'general') => void;
}

const TOOL_PRESETS = [
  { value: 'standard', label: 'Standard', description: 'Lean local toolset for general assistance', badge: 'Default' },
  { value: 'chat', label: 'Chat', description: 'Conversational assistant with minimal tool surface' },
  { value: 'research', label: 'Research', description: 'Live search and fetch tools' },
  { value: 'coding', label: 'Coding', description: 'Code editing, execution, and programmatic tools' },
];

export function ToolsSettings({ scopeLabel, conversationId, selected, onChange, selectedTools = [], onToolsChange, accessMode = 'ask', interactionMode = 'act', onInteractionModeChange, preset = 'standard', onPresetChange, busy, loading = false, saving = false, onNavigate }: ToolsSettingsProps) {
  const [items, setItems] = useState<ConnectorView[]>([]);
  const [activeSkills, setActiveSkills] = useState<string[]>([]);
  const [workspace, setWorkspace] = useState('');
  const [hasDaytona, setHasDaytona] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState('');
  const [pending, setPending] = useState(true);
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [isolationMode, setIsolationMode] = useState<'unsandboxed' | 'enforced'>('unsandboxed');
  useEffect(() => {
    if (!nativeAvailable) { setPending(false); return; }
    let disposed = false;
    let remaining = 4;
    const settle = () => { remaining -= 1; if (remaining === 0 && !disposed) setPending(false); };
    const fail = (e: unknown) => { if (!disposed) setError(current => current || errorMessage(e)); };
    api.hasDaytonaKey().then(value => { if (!disposed) setHasDaytona(value); }).catch(fail).finally(settle);
    api.listConnectors().then(value => { if (!disposed) setItems(value); }).catch(fail).finally(settle);
    api.listSkills(conversationId).then(value => { if (!disposed) setActiveSkills(value.filter(item => item.active).map(item => item.id)); }).catch(fail).finally(settle);
    api.getWorkspace().then(value => { if (!disposed) setWorkspace(value.path); }).catch(fail).finally(settle);
    api.profiles().then(value => { if (!disposed) setProfiles(value); }).catch(() => {});
    api.isolationGet().then(value => { if (!disposed && value?.mode) setIsolationMode(value.mode); }).catch(() => {});
    return () => { disposed = true; };
  }, [conversationId]);
  const [effective, setEffective] = useState<EffectiveTool[] | null>(null);
  // The backend recomputes exactly what the next turn would offer — preset,
  // selections, capability gates, permission and interaction mode — so the
  // summary can never claim tools are off while a callable tool exists.
  // Selections save before these props change, so the refetch reads the
  // updated conversation tools.
  const selectionKey = `${selected.join(',')}|${selectedTools.map(t => `${t.connectorId}:${t.toolName}`).join(',')}`;
  useEffect(() => {
    if (!nativeAvailable || loading) return;
    let disposed = false;
    api.effectiveCapabilities(conversationId ?? '', preset, interactionMode)
      .then(tools => { if (!disposed) setEffective(tools); })
      .catch(() => { if (!disposed) setEffective(null); });
    return () => { disposed = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversationId, preset, interactionMode, accessMode, selectionKey, loading, saving]);
  const connectedItems = items.filter(item => item.connected);
  const toolCount = selectedTools.length + (selected.includes('__workspace') ? 5 : 0) + (selected.includes('__execution') ? 1 : 0) + (selected.includes('__daytona') ? 1 : 0) + (activeSkills.length ? 1 : 0);
  const workspaceMissing = !pending && !workspace;
  const daytonaMissing = !pending && !hasDaytona;
  const toolUnavailable = (tool: ToolSelection) => !pending && !connectedItems.some(item => item.id === tool.connectorId && item.tools.some(available => available.name === tool.toolName));
  const unavailableCount = (selected.includes('__workspace') && workspaceMissing ? 1 : 0)
    + (selected.includes('__execution') && workspaceMissing ? 1 : 0)
    + (selected.includes('__daytona') && daytonaMissing ? 1 : 0)
    + selectedTools.filter(toolUnavailable).length;
  const callable = effective?.filter(tool => tool.available && tool.permittedInMode) ?? null;
  const approvalCount = effective?.filter(tool => tool.available && tool.permittedInMode && tool.requiresApproval).length ?? 0;
  // "Unavailable" counts things the user turned on: preset tools the
  // interaction mode blocks, plus selections whose setup is missing.
  const blockedCount = (effective?.filter(tool => tool.enabled && (!tool.available || !tool.permittedInMode)).length ?? 0) + unavailableCount;
  const summary = loading ? 'Tools · Loading…'
    : effective ? `Tools · ${callable?.length ?? 0} available`
    : toolCount ? `Tools · ${toolCount} selected` : 'Tools · Off';
  const summaryDetails = effective
    ? `${approvalCount ? ` · ${approvalCount} need approval` : ''}${blockedCount ? ` · ${blockedCount} unavailable` : ''}${activeSkills.length ? ` · ${activeSkills.length} active skills` : ''}${saving ? ' · Saving…' : ''}`
    : `${unavailableCount ? ` · ${unavailableCount} unavailable` : ''}${activeSkills.length ? ` · ${activeSkills.length} active skills` : ''}${saving ? ' · Saving…' : ''}`;
  const effectiveStatus = (tool: EffectiveTool) =>
    !tool.available ? (tool.blockedReason ?? 'Unavailable')
    : !tool.permittedInMode ? (tool.blockedReason ?? `Blocked in ${interactionMode} mode`)
    : tool.requiresApproval ? 'Needs approval each run' : 'Ready';
  async function chooseWorkspace() {
    setChoosing(true); setError('');
    try { const path = await open({ directory: true, multiple: false, title: 'Choose workspace folder' }); if (typeof path === 'string') { await api.setWorkspace(path); setWorkspace(path); } }
    catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(false); }
  }
  const toggle = (id: string, on: boolean) => onChange(on ? [...selected, id] : selected.filter(item => item !== id));
  const permission = PERMISSION_OPTIONS.find(option => option.key === accessMode) ?? PERMISSION_OPTIONS[0];
  return <div className="settings-page tools-settings">
    <SettingsHeader title="Tools" description="Choose what your assistant can use to work with files and run code." />
    <div className="settings-callout">
      <strong>{scopeLabel ? `Editing tools for: ${scopeLabel}` : 'Tools for your next conversation'}</strong>
      <p>Changes save automatically and become the defaults for new chats. Other existing chats keep their own choices.</p>
      <p className="tools-summary" aria-live="polite">{summary}{summaryDetails}</p>
      {effective && effective.length > 0 && <details className="tools-effective">
        <summary>Tool details</summary>
        <ul className="tools-effective-list">
          {effective.map(tool => (
            <li key={tool.name} className={tool.available && tool.permittedInMode ? '' : 'tool-blocked'}>
              <span className="tools-effective-name">{tool.name}</span>
              <span className="tools-effective-source">{tool.source}</span>
              <span className="tools-effective-status">{effectiveStatus(tool)}</span>
            </li>
          ))}
        </ul>
      </details>}
    </div>
    {unavailableCount > 0 && <p className="tool-unavailable">Remove unavailable selections below or complete their setup before sending a message.</p>}
    {profiles.length > 0 && <SettingsGroup title="Setup profile" description="A profile picks a preset and interaction mode; the tools it enables are listed below. Recommendations come from the model's last qualification — not marketing labels.">
      {profiles.map(profile => (
        <SettingRow key={profile.id} label={profile.label}
          description={<>
            <span>{profile.preset} preset · {profile.interactionMode} mode</span>
            {profile.note && <span className="profile-note">{profile.note}</span>}
          </>}>
          <div className="profile-choice">
            {profile.recommended && <span className="profile-badge">Recommended</span>}
            <button type="button" className="secondary" disabled={busy}
              aria-pressed={preset === profile.preset && interactionMode === profile.interactionMode}
              onClick={() => { onPresetChange?.(profile.preset); onInteractionModeChange?.(profile.interactionMode); }}>
              {preset === profile.preset && interactionMode === profile.interactionMode ? 'Selected' : 'Use profile'}
            </button>
          </div>
        </SettingRow>
      ))}
    </SettingsGroup>}
    <SettingsGroup title="Workspace" description="Choose a folder, then enable file access so your assistant can read and edit files there.">
      <SettingRow label="Workspace files" description="Read, search, and edit files inside the folder.">
        <input type="checkbox" className="switch" aria-label="Workspace files" checked={selected.includes('__workspace')} disabled={busy || pending || (!selected.includes('__workspace') && (!workspace || toolCount + 5 > 32))} onChange={event => toggle('__workspace', event.target.checked)} />
      </SettingRow>
      <SettingRow label="Folder" description={workspace
        ? <span className="workspace-choice"><strong>{workspaceName(workspace)}</strong><details className="workspace-path"><summary>Full path</summary><code>{workspace}</code></details></span>
        : pending ? 'Checking for a workspace folder…' : 'Choose a folder to let the assistant read and edit files there.'}>
        <button type="button" className="secondary" disabled={!nativeAvailable || busy || choosing} onClick={() => void chooseWorkspace()}>{workspace ? 'Change folder' : 'Choose folder'}</button>
      </SettingRow>
    </SettingsGroup>
    <SettingsGroup title="Code execution" description={isolationMode === 'enforced'
      ? 'Local code runs in an AppContainer confined to the workspace (Settings → Execution). Cloud execution cannot be started.'
      : 'Local code runs unsandboxed, with this Windows account’s files and network. Cloud execution cannot be started.'}>
      <SettingRow label="Local code" description={`${isolationMode === 'enforced' ? 'Sandboxed · AppContainer' : 'Unsandboxed'} · ${accessMode === 'fullAccess' ? 'runs without prompts' : 'approval required'}`}>
        <input type="checkbox" className="switch" aria-label="Local code" checked={selected.includes('__execution')} disabled={busy || pending || (!selected.includes('__execution') && (!workspace || toolCount >= 32))} onChange={event => toggle('__execution', event.target.checked)} />
      </SettingRow>
      {selected.includes('__execution') && workspaceMissing && <p className="tool-unavailable setting-row-note" role="status">Selected but unavailable: choose a workspace folder above.</p>}
      <SettingRow label="Daytona cloud code" description="Unavailable. A saved key is left in place. This does not start a sandbox.">
        <input type="checkbox" className="switch" aria-label="Daytona cloud code" checked={selected.includes('__daytona')} disabled={busy || pending || !selected.includes('__daytona')} onChange={event => { if (!event.target.checked) toggle('__daytona', false); }} />
      </SettingRow>
      {selected.includes('__daytona') && <p className="tool-unavailable setting-row-note">This chat still has cloud execution selected. Uncheck it. Historical records stay readable, and no new sandbox is created.</p>}
    </SettingsGroup>
    <SettingsGroup title="Tool profile" description="Controls which harness tools are offered, without changing custom connector choices.">
      <RadioCards name="tool-preset" label="Tool preset" value={preset} options={TOOL_PRESETS} disabled={busy || !onPresetChange} onChange={value => onPresetChange?.(value)} />
    </SettingsGroup>
    <SettingsGroup title="Permissions">
      <SettingRow label={permission.label} description="Public web read, workspace read and write, local code, and shell are approved separately. The model cannot grant itself access.">
        {onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('general')}>Change</button>}
      </SettingRow>
    </SettingsGroup>
    {selectedTools.length > 0 && <SettingsGroup title="Selected connector tools">
      {selectedTools.map(tool => <SettingRow key={`${tool.connectorId}:${tool.toolName}`} label={`${tool.connectorId} · ${tool.toolName}${toolUnavailable(tool) ? ' · Not connected' : ''}`}>
        <button type="button" className="secondary" aria-label={`Remove ${tool.connectorId} · ${tool.toolName}`} disabled={busy || !onToolsChange} onClick={() => onToolsChange?.(selectedTools.filter(item => item.connectorId !== tool.connectorId || item.toolName !== tool.toolName))}>Remove</button>
      </SettingRow>)}
    </SettingsGroup>}
    {activeSkills.length > 0 && <p className="settings-note">Active skills: {activeSkills.join(', ')}. One tool enables reading their package files. Deactivate skills in Skills to remove it.</p>}
    <SettingsGroup title="Connected services">
      <SettingRow label={connectedItems.length ? `${connectedItems.length} connected` : 'None connected'} description={`${connectedItems.length ? `Connected: ${connectedItems.map(item => connectorName(item)).join(', ')}.` : 'No services connected yet.'} Add or reconnect services in Connectors.`}>
        {onNavigate && <button type="button" className="secondary" onClick={() => onNavigate('connectors')}>Manage connectors</button>}
      </SettingRow>
    </SettingsGroup>
    {error && <p role="alert" className="error">{error}</p>}
  </div>;
}

export function ApprovalDialog({ request, onResolve, onResolveAskUser }: { request: ToolApproval; onResolve: (allow: boolean) => Promise<void>; onResolveAskUser?: (choice: string | null) => Promise<void> }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [freeform, setFreeform] = useState('');
  const [isolationMode, setIsolationMode] = useState<'unsandboxed' | 'enforced'>('unsandboxed');
  useEffect(() => { dialog.current?.showModal(); }, []);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.isolationGet().then(value => { if (!disposed) setIsolationMode(value.mode); }).catch(() => {});
    return () => { disposed = true; };
  }, []);
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
      <h2 id="approval-title">{question || 'The assistant has a question'}</h2>
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
    <h2 id="approval-title">{request.localServerName ? `Allow local server ${request.localServerName} to run this tool?` : request.connector === 'Local execution' ? 'Run this code on your computer?' : `Allow ${request.connector} to run this tool?`}</h2>
    {request.localServerName && <p>This local server runs with your account’s file and network permissions. Review the arguments before allowing this action. Server ID: <code>{request.connector}</code></p>}
    <p><strong>{request.name}</strong></p><p>{request.connector === 'Local execution' ? (isolationMode === 'enforced'
      ? 'This code runs inside a Windows AppContainer confined to the workspace folder. Review the complete code before allowing it.'
      : 'This code runs unsandboxed with your Windows account’s permissions. It can access files outside the workspace and the network. Review the complete code before allowing it.') : request.connector === 'Daytona' ? 'Cloud execution is unavailable. This call will not be sent. Historical cloud records stay readable.' : request.connector === 'Workspace' ? 'This action will access the workspace folder on your computer using the arguments below.' : request.connector === 'Skills' ? 'Read a verified file from an active skill’s local package. This does not execute scripts or contact a service.' : 'The arguments below will be sent to this service. The action may read or change external data.'}</p>
    {(request.connector === 'Local execution' || request.connector === 'Daytona') && typeof request.arguments.code === 'string' ? <><p>Language: {String(request.arguments.language)} · Timeout: {String(request.arguments.timeout_seconds ?? 60)} seconds</p><pre className="approval-code">{request.arguments.code}</pre><details><summary>Full request</summary><pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre></details></> : <pre className="approval-arguments">{JSON.stringify(request.arguments, null, 2)}</pre>}
    {error && <p role="alert">{error}</p>}
    <div className="connector-actions"><button type="button" autoFocus className="secondary" disabled={busy} onClick={() => void resolve(false)}>Deny</button><button type="button" className="primary" disabled={busy} onClick={() => void resolve(true)}>Allow once</button></div>
  </dialog>;
}
