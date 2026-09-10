import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalDialog, ToolPicker } from './ToolControls';
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
  return <><ToolPicker selected={selected} onChange={setSelected} selectedTools={tools} onToolsChange={setTools} busy={false} /><output data-testid="selection">{JSON.stringify(tools)}</output></>;
}
describe('individual connector tools', () => {
  it('does not submit the chat when Enter is pressed in tool search', async () => {
    const onSubmit = vi.fn(event => event.preventDefault());
    render(<form onSubmit={onSubmit}><Picker /><button type="submit">Send</button></form>);
    await userEvent.click(screen.getByText('Tools · Off'));
    await userEvent.type(await screen.findByLabelText('Search available tools'), 'tool{Enter}');
    expect(onSubmit).not.toHaveBeenCalled();
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
      render(<ToolPicker selected={[]} onChange={vi.fn()} selectedTools={Array.from({length:31}, (_,index) => ({connectorId:'example',toolName:`tool_${index}`}))} onToolsChange={vi.fn()} busy={false} />);
      await userEvent.click(await screen.findByText('Tools · 32/32 enabled · 1 active skills'));
      expect(screen.getByText(/One tool enables reading their package files/)).toBeInTheDocument();
      await userEvent.click(await screen.findByText('example'));
      expect(screen.getByRole('checkbox', {name:'tool_31'})).toBeDisabled();
      expect(screen.getByRole('checkbox', {name:'tool_0'})).not.toBeDisabled();
      expect(screen.getByRole('checkbox', {name:'Workspace files'})).toBeDisabled();
    } finally { fixtures.skills = []; }
  });
  it('selects a specific tool from a large catalog and retains it when searching', async () => {
    render(<Picker />);
    await userEvent.click(screen.getByText('Tools · Off'));
    await userEvent.click(await screen.findByText('example'));
    await userEvent.click(screen.getByRole('checkbox', { name: 'example' }));
    expect(screen.getByRole('alert')).toHaveTextContent('at most 32');
    expect(screen.getByTestId('selection')).toHaveTextContent('[]');
    await userEvent.click(screen.getByRole('checkbox', { name: 'tool_39' }));
    expect(screen.getByTestId('selection')).toHaveTextContent('[{"connectorId":"example","toolName":"tool_39"}]');
    await userEvent.type(screen.getByRole('searchbox'), 'tool_3');
    expect(screen.getByRole('checkbox', { name: 'tool_39' })).toBeChecked();
    expect(screen.queryByRole('checkbox', { name: 'tool_0' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('checkbox', { name: 'Workspace files' }));
    expect(screen.getByText('Tools · 6/32 enabled')).toBeInTheDocument();
  });
  it('explains that choices are remembered for new chats and shows the workspace name and path', async () => {
    render(<Picker />);
    await userEvent.click(await screen.findByText('Tools · Off'));
    expect(screen.getByText(/Tool choices are remembered for new chats. Existing chats keep their own selections./)).toBeInTheDocument();
    expect(screen.getByText('Workspace', { selector: 'h4' })).toBeInTheDocument();
    expect(screen.getByText('Code execution', { selector: 'h4' })).toBeInTheDocument();
    expect(screen.getByText('Connected services', { selector: 'h4' })).toBeInTheDocument();
    expect(screen.getByText('workspace', { selector: '.workspace-choice strong' })).toBeInTheDocument();
    await userEvent.click(screen.getByText('Full path'));
    expect(screen.getByText('C:/workspace', { selector: '.workspace-path code' })).toBeInTheDocument();
  });
  it('marks group selection state and keeps searching per tool', async () => {
    render(<Picker />);
    await userEvent.click(await screen.findByText('Tools · Off'));
    await userEvent.click(await screen.findByText('example'));
    const all = screen.getByRole('checkbox', { name: 'example' });
    expect(all).not.toBeChecked();
    await userEvent.click(screen.getByRole('checkbox', { name: 'tool_0' }));
    expect(screen.getByText('1/40 selected')).toBeInTheDocument();
    expect(all).not.toBeChecked();
  });
  it('keeps unavailable selections visible with a reason until removal is explicit', async () => {
    const onToolsChange = vi.fn();
    render(<ToolPicker selected={[]} onChange={vi.fn()} selectedTools={[
      { connectorId: 'example', toolName: 'tool_39' },
      { connectorId: 'gone', toolName: 'lost_tool' },
    ]} onToolsChange={onToolsChange} busy={false} />);
    await expect(await screen.findByText('Tools · 2/32 enabled · 1 unavailable')).toBeVisible();
    expect(screen.getByText(/connector is no longer configured/)).toBeInTheDocument();
    // The selection survives; only an explicit removal changes it.
    expect(onToolsChange).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Remove unavailable tools' }));
    expect(onToolsChange).toHaveBeenCalledExactlyOnceWith([{ connectorId: 'example', toolName: 'tool_39' }]);
  });
  it('offers a reconnect action for a selected tool of a disconnected service', async () => {
    const offline = { id: 'offline', connected: false, hasCredential: true, description: '', url: '', authType: 'apiKey', tools: [{ name: 'their_tool', description: '', inputSchema: {} }] };
    fixtures.connectors = [...fixtures.connectors, offline];
    try {
      render(<ToolPicker selected={[]} onChange={vi.fn()} selectedTools={[{ connectorId: 'offline', toolName: 'their_tool' }]} onToolsChange={vi.fn()} busy={false} />);
      await userEvent.click(await screen.findByText('Tools · 1/32 enabled · 1 unavailable'));
      expect(screen.getByText('offline')).toBeInTheDocument();
      expect(screen.getByText(/Not connected/)).toBeInTheDocument();
      await userEvent.click(screen.getByRole('button', { name: 'Reconnect' }));
      // Reconnecting restores availability: the service offers the tool again.
      expect(await screen.findByText('1/1 selected')).toBeVisible();
      expect(screen.getByText('Tools · 1/32 enabled')).toBeInTheDocument();
      expect(screen.queryByText(/Not connected/)).not.toBeInTheDocument();
    } finally { fixtures.connectors = [fixtures.connectors[0]]; }
  });
});
