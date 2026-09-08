import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ToolPicker } from './ToolControls';
import type { ToolSelection } from '../lib/types';
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: {
  listConnectors: async () => [{ id: 'example', connected: true, tools: Array.from({ length: 40 }, (_, index) => ({ name: `tool_${index}`, description: `Action ${index}`, inputSchema: {} })) }],
  listSkills: async () => [], getWorkspace: async () => ({ path: 'C:/workspace' }),
} }));
function Picker() {
  const [selected, setSelected] = useState<string[]>([]);
  const [tools, setTools] = useState<ToolSelection[]>([]);
  return <><ToolPicker selected={selected} onChange={setSelected} selectedTools={tools} onToolsChange={setTools} busy={false} /><output data-testid="selection">{JSON.stringify(tools)}</output></>;
}
describe('individual connector tools', () => {
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
    expect(screen.getByText('Tools · 5/32 enabled')).toBeInTheDocument();
  });
});
