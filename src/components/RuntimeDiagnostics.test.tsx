import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RuntimeDiagnostics } from './RuntimeDiagnostics';
const read = vi.hoisted(() => vi.fn());
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: { readRuntimeLog: read } }));
it('renders log text literally and refreshes after an error', async () => {
  read.mockRejectedValueOnce('Read failed').mockResolvedValueOnce({ content: '<script>example</script>', truncated: true });
  render(<RuntimeDiagnostics />);
  expect(await screen.findByRole('alert')).toHaveTextContent('Read failed');
  await userEvent.click(screen.getByRole('button', { name: 'Refresh log' }));
  expect(await screen.findByLabelText('Runtime log output')).toHaveTextContent('<script>example</script>');
  expect(screen.getByRole('status')).toHaveTextContent('final 64 KiB');
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
