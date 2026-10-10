import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { TaskReview } from './TaskReview';
import type { TaskReview as TaskReviewData, TaskWorkspace } from '../lib/types';

const task: TaskWorkspace = {
  id: 'task-1',
  conversationId: 'c1',
  sourceRoot: 'C:/repo',
  kind: 'git-worktree',
  isolatedRoot: 'C:/app/task-workspaces/task-1',
  branch: 'localllm/task-aa11bb22',
  baselineCommit: 'abc123',
  status: 'active',
  exclusions: [{ path: '.env', reason: 'looks like a secret; never copied into task workspaces' }],
  createdAt: 1,
  updatedAt: 1,
};

const review: TaskReviewData = {
  taskId: 'task-1',
  sourceRoot: 'C:/repo',
  isolatedRoot: 'C:/app/task-workspaces/task-1',
  kind: 'git-worktree',
  branch: 'localllm/task-aa11bb22',
  baselineCommit: 'abc123',
  exclusions: task.exclusions,
  files: [
    { path: 'src/app.ts', oldPath: null, status: 'modified', binary: false, size: 120, baselineHash: 'a', taskHash: 'b', destinationHash: 'a', destinationChanged: false, conflict: false },
    { path: 'src/lib.ts', oldPath: null, status: 'modified', binary: false, size: 60, baselineHash: 'a', taskHash: 'b', destinationHash: 'c', destinationChanged: true, conflict: true },
    { path: 'notes.md', oldPath: null, status: 'added', binary: false, size: 5, baselineHash: null, taskHash: 'b', destinationHash: null, destinationChanged: false, conflict: false },
  ],
  checkpoints: [{ commit: 'deadbeef', label: 'checkpoint: before run_code', createdAt: 2, uncommitted: false }],
  applies: [],
};

const mocks = vi.hoisted(() => ({
  status: vi.fn(async () => task as TaskWorkspace | null),
  review: vi.fn(async () => review),
  diff: vi.fn(async () => ({ text: 'diff --git a/src/app.ts\n+changed', truncated: false, binary: false, reason: null })),
  apply: vi.fn(async () => ({ applied: ['src/app.ts'], keptDestination: [], skippedConflicts: ['src/lib.ts'], backupDir: 'C:/recovery/1', applyId: 'a1' })),
  undo: vi.fn(async () => ({ restored: ['src/app.ts'], conflicts: [] })),
  discard: vi.fn(async () => {}),
  restore: vi.fn(async () => {}),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => String(e),
  api: {
    taskWorkspaceStatus: mocks.status,
    taskReview: mocks.review,
    taskFileDiff: mocks.diff,
    taskApply: mocks.apply,
    taskUndoApply: mocks.undo,
    taskDiscard: mocks.discard,
    taskRestoreCheckpoint: mocks.restore,
  },
}));

function renderReview() {
  return render(<TaskReview conversation="c1" refreshTick={0} onStatusChanged={() => {}} />);
}

describe('task review view', () => {
  it('renders nothing when the conversation has no isolated task', async () => {
    mocks.status.mockResolvedValueOnce(null);
    const { container } = renderReview();
    await waitFor(() => expect(mocks.status).toHaveBeenCalledWith('c1'));
    expect(container.querySelector('.task-review')).toBeNull();
  });

  it('lists changes with status badges and marks conflicts', async () => {
    renderReview();
    expect(await screen.findByText('src/app.ts')).toBeInTheDocument();
    expect(screen.getByText('notes.md')).toBeInTheDocument();
    expect(screen.getByText('conflict')).toBeInTheDocument();
    // An unresolved conflict holds back Apply all.
    expect(screen.getByRole('button', { name: /Apply all/ })).toBeDisabled();
    expect(screen.getByText(/changes/, { selector: '.task-review-summary' }).textContent).toContain('3 changes');
    expect(screen.getByText(/changes/, { selector: '.task-review-summary' }).textContent).toContain('1 added · 2 modified · 0 deleted');
  });

  it('applies once conflicts get a resolution and reports the result', async () => {
    renderReview();
    const applyAll = await screen.findByRole('button', { name: /Apply all/ });
    expect(applyAll).toBeDisabled();
    fireEvent.change(screen.getByRole('combobox', { name: 'Resolve conflict for src/lib.ts' }), { target: { value: 'take-task' } });
    await waitFor(() => expect(applyAll).toBeEnabled());
    fireEvent.click(applyAll);
    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith('task-1', undefined, { 'src/lib.ts': 'take-task' }));
    await screen.findByText(/1 file applied\./);
  });

  it('applies only the selected files', async () => {
    renderReview();
    await screen.findByText('src/app.ts');
    fireEvent.change(screen.getByRole('combobox', { name: 'Resolve conflict for src/lib.ts' }), { target: { value: 'keep-destination' } });
    fireEvent.click(screen.getByRole('checkbox', { name: 'Select notes.md' }));
    fireEvent.click(screen.getByRole('button', { name: /Apply selected/ }));
    await waitFor(() => expect(mocks.apply).toHaveBeenCalledWith('task-1', ['notes.md'], { 'src/lib.ts': 'keep-destination' }));
  });

  it('shows the file diff and truncation/binary notices', async () => {
    renderReview();
    fireEvent.click(await screen.findByText('src/app.ts'));
    await waitFor(() => expect(mocks.diff).toHaveBeenCalledWith('task-1', 'src/app.ts'));
    expect(await screen.findByText(/\+changed/)).toBeInTheDocument();
  });

  it('undoes the last apply through its record', async () => {
    mocks.review.mockResolvedValueOnce({
      ...review,
      applies: [{ id: 'a1', taskId: 'task-1', createdAt: 3, backupDir: 'C:/recovery/1', files: [{ path: 'src/app.ts', action: 'written', wroteSha256: 'b', backup: 'backup/src/app.ts', taskVersion: null }], undoneAt: null }],
    });
    renderReview();
    fireEvent.click(await screen.findByRole('button', { name: /Undo last apply/ }));
    await waitFor(() => expect(mocks.undo).toHaveBeenCalledWith('task-1', 'a1'));
    await screen.findByText(/Restored 1 file\./);
  });

  it('confirms discard before removing the workspace', async () => {
    renderReview();
    fireEvent.click(await screen.findByRole('button', { name: /Discard task/ }));
    fireEvent.click(screen.getByRole('button', { name: 'Discard workspace' }));
    await waitFor(() => expect(mocks.discard).toHaveBeenCalledWith('task-1'));
  });
});
