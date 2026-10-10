import { describe, expect, it, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { Recipe, Task } from '../lib/types';

const tasks: Task[] = [
  {
    id: 't-run', conversationId: 'c1', content: 'compile the docs', interactionMode: 'act',
    status: 'running', position: 1, runId: 'r1', waitReason: null, error: null,
    nextAction: null, parentTaskId: null, createdAt: 1, startedAt: 2, endedAt: null,
  },
  {
    id: 't-wait', conversationId: 'c1', content: 'rebuild the index', interactionMode: 'act',
    status: 'waiting_resource', position: 2, runId: null, waitReason: 'task t-run · compile the docs',
    error: null, nextAction: null, parentTaskId: null, createdAt: 3, startedAt: null, endedAt: null,
  },
  {
    id: 't-review', conversationId: 'c2', content: 'edit config', interactionMode: 'act',
    status: 'awaiting_review', position: null, runId: 'r2', waitReason: null, error: null,
    nextAction: 'Call call-2 (file_write) may have run — review before resuming.',
    parentTaskId: null, createdAt: 4, startedAt: null, endedAt: null,
  },
  {
    id: 't-failed', conversationId: 'c2', content: 'run the suite', interactionMode: 'act',
    status: 'failed', position: null, runId: null, waitReason: null,
    error: 'model timed out', nextAction: null, parentTaskId: null, createdAt: 5, startedAt: 6, endedAt: 7,
  },
];

const apiMocks = vi.hoisted(() => ({
  tasksList: vi.fn<() => Promise<Task[]>>(async () => []),
  taskPause: vi.fn(async () => {}),
  taskResume: vi.fn(async () => {}),
  taskCancel: vi.fn(async () => {}),
  taskRetry: vi.fn(async () => ({} as Task)),
  recipesList: vi.fn<() => Promise<Recipe[]>>(async () => []),
  recipeSave: vi.fn(async () => ({} as Recipe)),
  recipeDelete: vi.fn(async () => {}),
  recipeRunNow: vi.fn(async () => ({} as Task)),
}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: apiMocks,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

import { TaskInbox } from './TaskInbox';

describe('task inbox', () => {
  beforeEach(() => {
    for (const mock of Object.values(apiMocks)) mock.mockReset();
    apiMocks.tasksList.mockResolvedValue(tasks);
  });

  it('lists tasks with status chips, wait reasons and next actions', async () => {
    render(<TaskInbox onSelectConversation={() => {}} />);
    await screen.findByText('compile the docs');
    expect(screen.getByText('Running')).toBeInTheDocument();
    expect(screen.getByText('Waiting')).toBeInTheDocument();
    expect(screen.getByText(/task t-run/)).toBeInTheDocument();
    expect(screen.getByText('Needs review')).toBeInTheDocument();
    expect(screen.getByText(/call-2/)).toBeInTheDocument();
    expect(screen.getByText('Failed')).toBeInTheDocument();
    expect(screen.getByText('model timed out')).toBeInTheDocument();
  });

  it('runs pause, resume, retry and cancel actions', async () => {
    render(<TaskInbox onSelectConversation={() => {}} />);
    await screen.findByText('compile the docs');
    fireEvent.click(screen.getByRole('button', { name: /Pause task compile/ }));
    await waitFor(() => expect(apiMocks.taskPause).toHaveBeenCalledWith('t-run'));
    fireEvent.click(screen.getByRole('button', { name: /Cancel task compile/ }));
    await waitFor(() => expect(apiMocks.taskCancel).toHaveBeenCalledWith('t-run'));
    fireEvent.click(screen.getAllByRole('button', { name: /Retry task/ })[0]);
    await waitFor(() => expect(apiMocks.taskRetry).toHaveBeenCalled());
  });

  it('opens the task conversation when its title is clicked', async () => {
    const onSelect = vi.fn();
    render(<TaskInbox onSelectConversation={onSelect} />);
    fireEvent.click(await screen.findByTitle('compile the docs'));
    expect(onSelect).toHaveBeenCalledWith('c1');
  });

  it('collapses and expands the section', async () => {
    render(<TaskInbox onSelectConversation={() => {}} />);
    await screen.findByText('compile the docs');
    fireEvent.click(screen.getByRole('button', { name: /Tasks/ }));
    expect(screen.queryByText('compile the docs')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Tasks/ }));
    await screen.findByText('compile the docs');
  });

  it('lists, saves and runs recipes', async () => {
    apiMocks.recipesList.mockResolvedValue([{ id: 'r1', name: 'Tidy', prompt: 'clean locks', intervalMinutes: 15, enabled: true, nextRunAt: 1_700_000_000, lastRunAt: null, createdAt: 1 }]);
    render(<TaskInbox onSelectConversation={() => {}} />);
    fireEvent.click(await screen.findByRole('button', { name: /Recipes/ }));
    await screen.findByText('Tidy');
    expect(screen.getByText('every 15 min')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Run recipe Tidy' }));
    await waitFor(() => expect(apiMocks.recipeRunNow).toHaveBeenCalledWith('r1'));
    fireEvent.change(screen.getByLabelText('Recipe name'), { target: { value: 'Sweep' } });
    fireEvent.change(screen.getByLabelText('Recipe prompt'), { target: { value: 'remove temp files' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save recipe' }));
    await waitFor(() => expect(apiMocks.recipeSave).toHaveBeenCalledWith(expect.objectContaining({ name: 'Sweep', prompt: 'remove temp files', intervalMinutes: null, enabled: true })));
    fireEvent.click(screen.getByRole('button', { name: 'Delete recipe Tidy' }));
    await waitFor(() => expect(apiMocks.recipeDelete).toHaveBeenCalledWith('r1'));
  });
});
