import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalDialog, PermissionSelector, ToolsSettings } from './ToolControls';
import type { ToolSelection } from '../lib/types';
const fixtures = vi.hoisted(() => {
  const example = { id: 'example', connected: true, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: Array.from({ length: 40 }, (_, index) => ({ name: `tool_${index}`, description: `Action ${index}`, inputSchema: {} })) };
  const offline = { id: 'offline', connected: false, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: [{ name: 'their_tool', description: '', inputSchema: {} }] };
  return { skills: [] as { id: string; active: boolean }[], connectors: [example] as (typeof example | typeof offline)[] };
});
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  hasDaytonaKey: async () => false,
  listConnectors: async () => fixtures.connectors,
  connectConnector: async (id: string) => ({ ...fixtures.connectors.find(item => item.id === id)!, connected: true }),
  listSkills: async () => fixtures.skills, getWorkspace: async () => ({ path: 'C:/workspace' }),
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
    expect(screen.getByText('Safe')).toBeInTheDocument();
    expect(screen.getByText('Reads only')).toBeInTheDocument();
    expect(screen.getByText('Unrestricted')).toBeInTheDocument();

    const fullAccessOption = screen.getByRole('button', { name: /Full access/ });
    await userEvent.click(fullAccessOption);
    expect(onAccessModeChange).toHaveBeenCalledWith('fullAccess');
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();

    rerender(<PermissionSelector accessMode="fullAccess" onAccessModeChange={onAccessModeChange} />);
    const fullTrigger = screen.getByRole('button', { name: /Permission mode: Full access/ });
    expect(fullTrigger).toHaveClass('permission-trigger-full');

    await userEvent.click(fullTrigger);
    expect(screen.getByRole('dialog', { name: 'Select permission mode' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Select permission mode' })).not.toBeInTheDocument();
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
