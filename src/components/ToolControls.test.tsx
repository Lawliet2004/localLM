import { useState } from 'react';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalDialog, PermissionSelector, ToolsSettings } from './ToolControls';
import type { ToolSelection } from '../lib/types';
const fixtures = vi.hoisted(() => {
  const example = { id: 'example', connected: true, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: Array.from({ length: 40 }, (_, index) => ({ name: `tool_${index}`, description: `Action ${index}`, inputSchema: {} })) };
  const offline = { id: 'offline', connected: false, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: [{ name: 'their_tool', description: '', inputSchema: {} }] };
  return {
    skills: [] as { id: string; active: boolean }[],
    connectors: [example] as (typeof example | typeof offline)[],
    effective: null as { name: string; source: string; available: boolean; enabled: boolean; permittedInMode: boolean; requiresApproval: boolean; blockedReason?: string }[] | null,
  };
});
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
    profiles: async () => [],
    hasDaytonaKey: async () => false,
  listConnectors: async () => fixtures.connectors,
  connectConnector: async (id: string) => ({ ...fixtures.connectors.find(item => item.id === id)!, connected: true }),
  listSkills: async () => fixtures.skills, getWorkspace: async () => ({ path: 'C:/workspace' }),
  effectiveCapabilities: async () => fixtures.effective,
  isolationGet: async () => ({ mode: 'unsandboxed', network: false, lan: false, lpac: false, toolchainDirs: [], processLimit: 32, memoryMb: 2048 }),
} }));
function Picker() {
  const [selected, setSelected] = useState<string[]>([]);
  const [tools, setTools] = useState<ToolSelection[]>([]);
  return <><ToolsSettings selected={selected} onChange={setSelected} selectedTools={tools} onToolsChange={setTools} busy={false} /><output data-testid="selection">{JSON.stringify(tools)}</output></>;
}
describe('tool controls', () => {
  it('identifies and removes saved tools whose connector is disconnected', async () => {
    const onToolsChange = vi.fn();
    render(<ToolsSettings selected={[]} onChange={vi.fn()} selectedTools={[{ connectorId: 'offline', toolName: 'their_tool' }]} onToolsChange={onToolsChange} busy={false} />);
    await userEvent.click(await screen.findByText('Tools · 1 selected · 1 unavailable'));
    expect(screen.getByText('offline · their_tool · Not connected')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Remove offline · their_tool' }));
    expect(onToolsChange).toHaveBeenCalledWith([]);
  });
  it.each([['Allow once', true], ['Deny', false]] as const)('identifies a local server before %s', async (button, decision) => {
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable:true, value:function(this: HTMLDialogElement) { this.open = true; } });
    try {
      const onResolve = vi.fn().mockResolvedValue(undefined);
      render(<ApprovalDialog request={{id:'approval',connector:'local-stable-id',localServerName:'Research server',name:'lookup',arguments:{query:'private query'}}} onResolve={onResolve} />);
      expect(screen.getByRole('heading', {name:'Allow local server Research server to run this tool?'})).toBeVisible();
      expect(screen.getByText('local-stable-id')).toBeVisible();
      expect(screen.getByText(/account’s file and network permissions/)).toBeVisible();
      await userEvent.click(screen.getByRole('button', {name:button}));
      expect(onResolve).toHaveBeenCalledExactlyOnceWith(decision);
    } finally { if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original); else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal'); }
  });
  it('reserves a visible tool slot for active skill references', async () => {
    fixtures.skills = [{ id: 'jupyter-notebook', active: true }];
    try {
      render(<ToolsSettings selected={[]} onChange={vi.fn()} selectedTools={Array.from({length:31}, (_,index) => ({connectorId:'example',toolName:`tool_${index}`}))} busy={false} />);
      await userEvent.click(await screen.findByText('Tools · 32 selected · 1 active skills'));
      expect(screen.getByText(/One tool enables reading their package files/)).toBeInTheDocument();
      expect(screen.getByRole('checkbox', {name:'Workspace files'})).toBeDisabled();
    } finally { fixtures.skills = []; }
  });
  it('explains that choices are remembered for new chats and shows the workspace name and path', async () => {
    render(<Picker />);
    await userEvent.click(await screen.findByText('Tools · Off'));
    expect(screen.getByText(/Changes save automatically and become the defaults for new chats./)).toBeInTheDocument();
    expect(screen.getByText('Workspace', { selector: 'h2' })).toBeInTheDocument();
    expect(screen.getByText('Code execution', { selector: 'h2' })).toBeInTheDocument();
    expect(screen.getByText('Connected services', { selector: 'h2' })).toBeInTheDocument();
    expect(screen.getByText('workspace', { selector: '.workspace-choice strong' })).toBeInTheDocument();
    await userEvent.click(screen.getByText('Full path'));
    expect(screen.getByText('C:/workspace', { selector: '.workspace-path code' })).toBeInTheDocument();
  });
  it('shows connected services note when connectors are connected', async () => {
    render(<Picker />);
    await userEvent.click(await screen.findByText('Tools · Off'));
    expect(await screen.findByText(/Connected: example\./)).toBeInTheDocument();
  });
  it('toggles workspace files and code execution options', async () => {
    const onChange = vi.fn();
    render(<ToolsSettings selected={[]} onChange={onChange} busy={false} />);
    await userEvent.click(await screen.findByText('Tools · Off'));
    const workspaceCheckbox = screen.getByRole('checkbox', { name: 'Workspace files' });
    await userEvent.click(workspaceCheckbox);
    expect(onChange).toHaveBeenCalledWith(['__workspace']);
    const localCheckbox = screen.getByRole('checkbox', { name: 'Local code' });
    await userEvent.click(localCheckbox);
    expect(onChange).toHaveBeenCalledWith(['__execution']);
  });
  it('keeps setup visible as a standalone page', async () => {
    render(<Picker />);
    expect(await screen.findByRole('heading', {name:'Tools'})).toBeVisible();
    await userEvent.keyboard('{Escape}');
    expect(screen.getByRole('checkbox', {name:'Workspace files'})).toBeVisible();
  });
  it('lets the user pick any ask_user option or type a freeform answer', async () => {
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable:true, value:function(this: HTMLDialogElement) { this.open = true; } });
    try {
      const onResolveAskUser = vi.fn().mockResolvedValue(undefined);
      render(<ApprovalDialog request={{id:'q1',kind:'ask_user',connector:'Interaction',name:'ask_user',arguments:{question:'Deploy now?',options:['today','tomorrow']}}} onResolve={vi.fn().mockResolvedValue(undefined)} onResolveAskUser={onResolveAskUser} />);
      expect(screen.getByRole('heading', {name:'Deploy now?'})).toBeVisible();
      await userEvent.click(screen.getByRole('button', {name:'tomorrow'}));
      expect(onResolveAskUser).toHaveBeenCalledExactlyOnceWith('tomorrow');
    } finally { if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original); else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal'); }
  });
  it('renders beautiful permission selector and toggles options', async () => {
    const onAccessModeChange = vi.fn();
    const { rerender } = render(<PermissionSelector accessMode="ask" onAccessModeChange={onAccessModeChange} />);
    const trigger = screen.getByRole('button', { name: /Permission mode: Ask for approval/ });
    expect(trigger).toBeInTheDocument();
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();

    await userEvent.click(trigger);
    const popover = screen.getByRole('dialog', { name: 'Select permission mode' });
    expect(popover).toBeInTheDocument();
    const options = within(popover).getAllByRole('button');
    expect(options).toHaveLength(3);
    expect(within(popover).getByRole('button', { name: /Ask for approval/ })).toHaveAttribute('aria-pressed', 'true');
    expect(within(popover).getByRole('button', { name: /Auto-approve reads/ })).toBeInTheDocument();
    expect(within(popover).getByRole('button', { name: /Full access/ })).toBeInTheDocument();

    await userEvent.click(within(popover).getByRole('button', { name: /Full access/ }));
    expect(onAccessModeChange).toHaveBeenCalledWith('fullAccess');
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();

    rerender(<PermissionSelector accessMode="fullAccess" onAccessModeChange={onAccessModeChange} />);
    const fullTrigger = screen.getByRole('button', { name: /Permission mode: Full access/ });
    expect(fullTrigger).toHaveClass('permission-trigger-full');

    await userEvent.click(fullTrigger);
    expect(screen.getByRole('dialog', { name: 'Select permission mode' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();

    // Outside pointerdown also closes the popover.
    await userEvent.click(fullTrigger);
    expect(screen.getByRole('dialog', { name: 'Select permission mode' })).toBeInTheDocument();
    await userEvent.click(document.body);
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();
  });
  it('summarizes backend-derived effective tools and lists each tool with its reason', async () => {
    fixtures.effective = [
      { name: 'read_file', source: 'workspace', available: true, enabled: true, permittedInMode: true, requiresApproval: false },
      { name: 'list_files', source: 'workspace', available: true, enabled: true, permittedInMode: true, requiresApproval: false },
      { name: 'edit_file', source: 'workspace', available: true, enabled: true, permittedInMode: false, requiresApproval: true, blockedReason: 'Plan mode only permits read-only tools.' },
      { name: 'web_search', source: 'harness', available: true, enabled: true, permittedInMode: true, requiresApproval: true },
      { name: 'browser', source: 'harness', available: false, enabled: false, permittedInMode: false, requiresApproval: true, blockedReason: "Not part of the 'standard' tool preset." },
    ];
    try {
      render(<ToolsSettings selected={[]} onChange={vi.fn()} busy={false} interactionMode="plan" />);
      expect(await screen.findByText('Tools · 3 available · 1 need approval · 1 unavailable')).toBeInTheDocument();
      await userEvent.click(screen.getByText('Tool details'));
      expect(screen.getByText('edit_file')).toBeInTheDocument();
      expect(screen.getByText('Plan mode only permits read-only tools.')).toBeInTheDocument();
      expect(screen.getByText("Not part of the 'standard' tool preset.")).toBeInTheDocument();
      expect(screen.getByText('web_search')).toBeInTheDocument();
      expect(screen.getAllByText('Needs approval each run')).toHaveLength(1);
    } finally { fixtures.effective = null; }
  });
  it('never says Off while the backend reports a callable tool', async () => {
    fixtures.effective = [
      { name: 'read_file', source: 'workspace', available: true, enabled: true, permittedInMode: true, requiresApproval: true },
    ];
    try {
      render(<ToolsSettings selected={[]} onChange={vi.fn()} busy={false} />);
      expect(await screen.findByText('Tools · 1 available · 1 need approval')).toBeInTheDocument();
      expect(screen.queryByText(/Tools · Off/)).not.toBeInTheDocument();
    } finally { fixtures.effective = null; }
  });
  it('renders tool profile preset selector and calls onPresetChange', async () => {
    const onPresetChange = vi.fn();
    render(
      <ToolsSettings
        selected={[]}
        onChange={vi.fn()}
        preset="standard"
        onPresetChange={onPresetChange}
        busy={false}
      />
    );
    expect(await screen.findByRole('radiogroup', { name: 'Tool preset' })).toBeInTheDocument();
    const chatRadio = screen.getByRole('radio', { name: /Chat/ });
    expect(chatRadio).not.toBeChecked();
    const standardRadio = screen.getByRole('radio', { name: /Standard/ });
    expect(standardRadio).toBeChecked();
    await userEvent.click(chatRadio);
    expect(onPresetChange).toHaveBeenCalledWith('chat');
  });
});
