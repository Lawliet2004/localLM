import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalDialog, PermissionSelector, ToolsSettings } from './ToolControls';
import type { ToolSelection } from '../lib/types';
const fixtures = vi.hoisted(() => {
  const example = { id: 'example', connected: true, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: Array.from({ length: 40 }, (_, index) => ({ name: `tool_${index}`, description: `Action ${index}`, inputSchema: {} })) };
  const offline = { id: 'offline', connected: false, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: [{ name: 'their_tool', description: '', inputSchema: {} }] };
  return { automation: { browserEnabled: false, computerEnabled: false }, skills: [] as { id: string; active: boolean }[], connectors: [example] as (typeof example | typeof offline)[] };
});
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  hasDaytonaKey: async () => false,
  getAutomationConfig: async () => fixtures.automation,
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
  it('counts the registered workspace search tools in source capacity', async () => {
    render(<ToolsSettings selected={['__workspace']} onChange={vi.fn()} busy={false} />);
    expect(await screen.findByText(/28 selected source tools/)).toBeInTheDocument();
    expect(screen.getByText('Read, search, and edit files in the workspace folder.')).toBeVisible();
  });
  it('routes automation setup to its dedicated settings pages', async () => {
    const onNavigate = vi.fn();
    render(<ToolsSettings selected={[]} onChange={vi.fn()} busy={false} onNavigate={onNavigate} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Configure browser use' }));
    expect(onNavigate).toHaveBeenLastCalledWith('browser-use');
    await userEvent.click(screen.getByRole('button', { name: 'Configure computer use' }));
    expect(onNavigate).toHaveBeenLastCalledWith('computer-use');
  });
  it('offers enabled automation as persisted source selections with explicit source counts', async () => {
    fixtures.automation = { browserEnabled: true, computerEnabled: true };
    function AutomationPicker() {
      const [selected, setSelected] = useState<string[]>([]);
      return <><ToolsSettings selected={selected} onChange={setSelected} busy={false} /><output data-testid="automation-sources">{JSON.stringify(selected)}</output></>;
    }
    try {
      render(<AutomationPicker />);
      const browser = await screen.findByRole('checkbox', { name: 'Browser use' });
      await userEvent.click(browser);
      expect(browser).toBeChecked();
      expect(screen.getByText('Tools · 1 source')).toBeInTheDocument();
      await userEvent.click(screen.getByRole('checkbox', { name: 'Computer use' }));
      expect(screen.getByText('Tools · 2 sources')).toBeInTheDocument();
      expect(screen.getByTestId('automation-sources')).toHaveTextContent('["__browser","__computer"]');
    } finally { fixtures.automation = { browserEnabled: false, computerEnabled: false }; }
  });

  it('identifies and removes saved tools whose connector is disconnected', async () => {
    const onToolsChange = vi.fn();
    render(<ToolsSettings selected={[]} onChange={vi.fn()} selectedTools={[{ connectorId: 'offline', toolName: 'their_tool' }]} onToolsChange={onToolsChange} busy={false} />);
    await userEvent.click(await screen.findByText('Tools · 1 connector tool · 1 unavailable'));
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
      render(<ToolsSettings selected={[]} onChange={vi.fn()} selectedTools={Array.from({length:63}, (_,index) => ({connectorId:'example',toolName:`tool_${index}`}))} busy={false} />);
      await userEvent.click(await screen.findByText(/Tools · 63 connector tools.*1 active skills/));
      expect(screen.getByText(/One tool enables reading their package files/)).toBeInTheDocument();
      expect(screen.getByRole('checkbox', {name:'Workspace files'})).toBeDisabled();
    } finally { fixtures.skills = []; }
  });
  it('explains that choices are remembered for new chats and shows the workspace name and path', async () => {
    render(<Picker />);
    await userEvent.click(await screen.findByText('Tools · No sources selected'));
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
    await userEvent.click(await screen.findByText('Tools · No sources selected'));
    expect(await screen.findByText(/Connected: example\./)).toBeInTheDocument();
  });
  it('toggles workspace files and code execution options', async () => {
    const onChange = vi.fn();
    render(<ToolsSettings selected={[]} onChange={onChange} busy={false} />);
    await userEvent.click(await screen.findByText('Tools · No sources selected'));
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
  it('never answers a blank ask_user form on behalf of the user', async () => {
    const onResolveAskUser = vi.fn().mockResolvedValue(undefined);
    const originalShow = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.setAttribute('open', ''); } });
    try {
      const { rerender } = render(<ApprovalDialog request={{id:'blank-1',kind:'ask_user',connector:'Interaction',name:'ask_user',arguments:{question:'Deploy now?',options:['today','tomorrow']}}} onResolve={vi.fn()} onResolveAskUser={onResolveAskUser} />);
      await userEvent.click(screen.getByRole('button', { name: 'Answer' }));
      expect(onResolveAskUser).not.toHaveBeenCalled();
      expect(screen.getByRole('alert')).toHaveTextContent('Choose an option or enter an answer.');
      rerender(<ApprovalDialog request={{id:'blank-2',kind:'ask_user',connector:'Interaction',name:'ask_user',arguments:{question:'What should happen?',options:[]}}} onResolve={vi.fn()} onResolveAskUser={onResolveAskUser} />);
      await userEvent.click(screen.getByRole('button', { name: 'Answer' }));
      expect(onResolveAskUser).not.toHaveBeenCalled();
    } finally {
      if (originalShow) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', originalShow);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal');
    }
  });
  it('does not let an MCP tool name select the ask_user interaction channel', async () => {
    const onResolve = vi.fn().mockResolvedValue(undefined);
    const onResolveAskUser = vi.fn().mockResolvedValue(undefined);
    const originalShow = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.setAttribute('open', ''); } });
    try {
      render(<ApprovalDialog request={{id:'mcp-ask-user',connector:'local-fixture',name:'ask_user',arguments:{question:'Server-controlled text',options:['send data']}}} onResolve={onResolve} onResolveAskUser={onResolveAskUser} />);
      expect(screen.queryByLabelText('Your answer')).not.toBeInTheDocument();
      await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
      expect(onResolve).toHaveBeenCalledExactlyOnceWith(false);
      expect(onResolveAskUser).not.toHaveBeenCalled();
    } finally {
      if (originalShow) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', originalShow);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal');
    }
  });
  it('renders a plan_approval dialog that approves or sends feedback', async () => {
    const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.open = true; } });
    try {
      const onResolveAskUser = vi.fn().mockResolvedValue(undefined);
      render(<ApprovalDialog request={{id:'plan-1',kind:'plan_approval',connector:'Interaction',name:'plan_submit',arguments:{plan:'## Plan\n1. Edit file',summary:'one step'}}} onResolve={vi.fn()} onResolveAskUser={onResolveAskUser} />);
      expect(screen.getByRole('heading', {name:'Review the proposed plan'})).toBeVisible();
      // The plan is rendered as markdown, not a raw string blob.
      expect(screen.getByText('Edit file')).toBeInTheDocument();
      // Request changes requires feedback, then resolves with the text.
      await userEvent.click(screen.getByRole('button', {name:'Request changes'}));
      expect(onResolveAskUser).not.toHaveBeenCalled();
      expect(screen.getByRole('alert')).toBeInTheDocument();
      await userEvent.type(screen.getByLabelText('Plan feedback'), 'Add tests first');
      await userEvent.click(screen.getByRole('button', {name:'Request changes'}));
      expect(onResolveAskUser).toHaveBeenCalledWith('Add tests first');
      onResolveAskUser.mockClear();
      // A resolved dialog latches busy; a fresh request approves cleanly.
      render(<ApprovalDialog request={{id:'plan-2',kind:'plan_approval',connector:'Interaction',name:'plan_submit',arguments:{plan:'## Plan\n1. Edit file'}}} onResolve={vi.fn()} onResolveAskUser={onResolveAskUser} />);
      await userEvent.click(screen.getAllByRole('button', {name:'Approve'})[1]);
      expect(onResolveAskUser).toHaveBeenCalledWith('approved');
    } finally {
      if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal');
    }
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
  it.each([['code', /^Code/], ['minimal', /^Minimal/], ['creator', /^Creator/]] as const)('selects and reflects the advanced %s preset without changing sources', async (id, name) => {
    const onChange = vi.fn();
    const onPresetChange = vi.fn();
    const { rerender } = render(<ToolsSettings selected={['__workspace']} onChange={onChange} preset="standard" onPresetChange={onPresetChange} busy={false} />);
    const radio = await screen.findByRole('radio', { name });
    await userEvent.click(radio);
    expect(onPresetChange).toHaveBeenCalledExactlyOnceWith(id);
    expect(onChange).not.toHaveBeenCalled();
    rerender(<ToolsSettings selected={['__workspace']} onChange={onChange} preset={id} onPresetChange={onPresetChange} busy={false} />);
    expect(screen.getByRole('radio', { name })).toBeChecked();
    rerender(<ToolsSettings selected={['__workspace']} onChange={onChange} preset={id} onPresetChange={onPresetChange} busy />);
    expect(screen.getByRole('radio', { name })).toBeDisabled();
  });
  describe('MCP input form', () => {
    const showModal = () => {
      const original = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, 'showModal');
      Object.defineProperty(HTMLDialogElement.prototype, 'showModal', { configurable: true, value: function(this: HTMLDialogElement) { this.setAttribute('open', ''); } });
      return () => { if (original) Object.defineProperty(HTMLDialogElement.prototype, 'showModal', original); else Reflect.deleteProperty(HTMLDialogElement.prototype, 'showModal'); };
    };
    const formRequest = { id: 'mcp-input-1', kind: 'mcp_input' as const, connector: 'local-stable-id', localServerName: 'Research server', name: '__locallm_mcp_resource_read', arguments: {}, form: {
      message: 'Choose a label',
      fields: [
        { name: 'label', kind: 'text' as const, required: true, title: 'Label' },
        { name: 'count', kind: 'integer' as const, minimum: 1, maximum: 9 },
        { name: 'color', kind: 'select' as const, options: [{ value: 'red', label: 'Red' }, { value: 'blue', label: 'Blue' }] },
      ],
    } };
    it('sends accepted form content and declines or cancels explicitly', async () => {
      const restore = showModal();
      try {
        const onResolve = vi.fn();
        const onResolveMcpInput = vi.fn().mockResolvedValue(undefined);
        render(<ApprovalDialog request={formRequest} onResolve={onResolve} onResolveMcpInput={onResolveMcpInput} />);
        expect(screen.getByRole('heading', { name: /asking for input/ })).toBeVisible();
        expect(screen.getByText('Choose a label')).toBeInTheDocument();
        await userEvent.type(screen.getByLabelText(/Label/), 'green');
        await userEvent.type(screen.getByLabelText(/count/), '4');
        await userEvent.selectOptions(screen.getByLabelText(/color/), 'blue');
        await userEvent.click(screen.getByRole('button', { name: 'Send' }));
        expect(onResolveMcpInput).toHaveBeenCalledExactlyOnceWith('accept', { label: 'green', count: 4, color: 'blue' });
        expect(onResolve).not.toHaveBeenCalled();
      } finally { restore(); }
    });
    it('blocks send until required fields are answered, without pre-filling a default', async () => {
      const restore = showModal();
      try {
        const onResolveMcpInput = vi.fn().mockResolvedValue(undefined);
        render(<ApprovalDialog request={formRequest} onResolve={vi.fn()} onResolveMcpInput={onResolveMcpInput} />);
        await userEvent.click(screen.getByRole('button', { name: 'Send' }));
        expect(onResolveMcpInput).not.toHaveBeenCalled();
        expect(screen.getByRole('alert')).toHaveTextContent('Label is required.');
        await userEvent.click(screen.getByRole('button', { name: 'Decline' }));
        expect(onResolveMcpInput).toHaveBeenCalledExactlyOnceWith('decline', undefined);
      } finally { restore(); }
    });
    it('routes cancel separately from decline', async () => {
      const restore = showModal();
      try {
        const onResolveMcpInput = vi.fn().mockResolvedValue(undefined);
        render(<ApprovalDialog request={formRequest} onResolve={vi.fn()} onResolveMcpInput={onResolveMcpInput} />);
        await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));
        expect(onResolveMcpInput).toHaveBeenCalledExactlyOnceWith('cancel', undefined);
      } finally { restore(); }
    });
    it('keeps the dialog open when the backend rejects an invalid answer', async () => {
      const restore = showModal();
      try {
        const onResolveMcpInput = vi.fn().mockRejectedValue(new Error('invalid field value'));
        render(<ApprovalDialog request={formRequest} onResolve={vi.fn()} onResolveMcpInput={onResolveMcpInput} />);
        await userEvent.type(screen.getByLabelText(/Label/), 'green');
        await userEvent.click(screen.getByRole('button', { name: 'Send' }));
        expect(onResolveMcpInput).toHaveBeenCalledExactlyOnceWith('accept', { label: 'green' });
        expect(await screen.findByRole('alert')).toHaveTextContent('invalid field value');
        expect(screen.getByLabelText(/Label/)).toBeInTheDocument();
      } finally { restore(); }
    });
  });
});
