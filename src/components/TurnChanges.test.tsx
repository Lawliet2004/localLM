import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { TurnChanges } from './TurnChanges';

const apiMocks = vi.hoisted(() => ({ listCheckpoints: vi.fn(), checkpointDiff: vi.fn(), revertCheckpoint: vi.fn(), checkpointSettings: vi.fn().mockResolvedValue({ enabled: true }), saveCheckpointSettings: vi.fn(), clearCheckpoints: vi.fn() }));
vi.mock('../lib/api', () => ({ api: apiMocks, nativeAvailable: true, errorMessage: String }));

const checkpoint = { id: 'c1', conversationId: 'conv', workspace: 'C:/ws', label: 'turn', beforeCommit: 'a', afterCommit: 'b', filesChanged: 2, excluded: [{ path: 'big.bin', reason: 'larger than 5 MB' }], status: 'complete', createdAt: 0 };

it('reviews a turn and reverts it only after confirmation, reporting conflicts', async () => {
  apiMocks.listCheckpoints.mockResolvedValue([checkpoint, { ...checkpoint, id: 'open', afterCommit: null }]);
  apiMocks.checkpointDiff.mockResolvedValue({ changes: [{ status: 'M', path: 'src/a.rs' }, { status: 'A', path: 'notes.md' }], diff: '+changed', truncated: false, excluded: [] });
  apiMocks.revertCheckpoint.mockResolvedValue({ restored: ['src/a.rs'], deleted: [], conflicts: ['notes.md'] });
  render(<TurnChanges conversationId="conv" revision="1" busy={false} />);
  expect(await screen.findByText(/2 file\(s\)/)).toBeInTheDocument();
  expect(screen.getAllByRole('button', { name: 'Revert' })).toHaveLength(1);
  expect(screen.getByText(/were not captured/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Review' }));
  expect(await screen.findByText('notes.md', { exact: false })).toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Revert' }));
  expect(apiMocks.revertCheckpoint).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole('button', { name: 'Confirm revert' }));
  expect(apiMocks.revertCheckpoint).toHaveBeenCalledWith('c1');
  expect(await screen.findByText(/edited after that turn: notes.md/)).toBeInTheDocument();
});

it('turning recording off saves the setting', async () => {
  apiMocks.listCheckpoints.mockResolvedValue([]);
  apiMocks.saveCheckpointSettings.mockResolvedValue({ enabled: false });
  render(<TurnChanges conversationId="conv" revision="1" busy={false} />);
  const toggle = await screen.findByLabelText('Record checkpoints');
  await vi.waitFor(() => expect(toggle).not.toBeDisabled());
  await userEvent.click(toggle);
  expect(apiMocks.saveCheckpointSettings).toHaveBeenCalledWith({ enabled: false });
  expect(await screen.findByLabelText('Record checkpoints')).not.toBeChecked();
});
