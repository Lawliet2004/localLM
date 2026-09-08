import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { LocalConnectorForm } from './LocalConnectorForm';
import { api } from '../lib/api';
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: { saveLocalConnector: vi.fn() }, errorMessage: (error: Error) => error.message }));
describe('local connector configuration', () => {
  it('saves exact argument boundaries and clears credentials without launching', async () => {
    vi.mocked(api.saveLocalConnector).mockResolvedValue();
    const onSaved = vi.fn().mockResolvedValue(undefined);
    render(<LocalConnectorForm onSaved={onSaved} />);
    fireEvent.click(screen.getByText('Add a local MCP server'));
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'Fixture' } });
    fireEvent.change(screen.getByLabelText('Executable path'), { target: { value: 'C:\\node.exe' } });
    fireEvent.change(screen.getByLabelText('Working directory'), { target: { value: 'C:\\work' } });
    fireEvent.change(screen.getByLabelText('Arguments (JSON array)'), { target: { value: '["path with spaces", ""]' } });
    fireEvent.change(screen.getByLabelText('Environment variables (JSON object)'), { target: { value: '{"TOKEN":"fixture-secret"}' } });
    fireEvent.click(screen.getByText('Save local server'));
    await waitFor(() => expect(onSaved).toHaveBeenCalledOnce());
    expect(api.saveLocalConnector).toHaveBeenCalledWith(expect.objectContaining({ name: 'Fixture', arguments: ['path with spaces', ''], environment: { TOKEN: 'fixture-secret' } }));
    expect(screen.getByLabelText('Environment variables (JSON object)')).toHaveValue('{}');
    expect(screen.getByRole('status')).toHaveTextContent('Use Connect');
  });
});
