import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ContextPanel } from './ContextPanel';
import type { ContextPreview, MemoryFact } from '../lib/types';

const preview: ContextPreview = {
  items: [
    { kind: 'rules', label: 'AGENTS.md', tokens: 40, reason: 'Project rules', source: '/repo/AGENTS.md' },
    { kind: 'repo-map', label: 'Repository map · 120 files', tokens: 900, reason: 'Workspace overview', source: '/repo' },
    { kind: 'memory', label: 'use pnpm not npm', tokens: 4, reason: 'user fact in project:p1', id: 'm1' },
    { kind: 'pinned', label: 'ship on Fridays', tokens: 4, reason: 'user fact in user', id: 'm2' },
  ],
  totalTokens: 948,
  budgetTokens: 2350,
};

const facts: MemoryFact[] = [
  { id: 'm1', scope: 'project:p1', fact: 'use pnpm not npm', origin: 'user', reason: 'CI uses pnpm', sourcePath: '/repo/package.json', pinned: false, excluded: false, stale: false, createdAt: 1, updatedAt: 2 },
  { id: 'm2', scope: 'user', fact: 'ship on Fridays', origin: 'agent', reason: null, sourcePath: null, pinned: true, excluded: false, stale: true, createdAt: 1, updatedAt: 2 },
  { id: 'm3', scope: 'conversation:c1', fact: 'draft only', origin: 'agent', reason: null, sourcePath: null, pinned: false, excluded: true, stale: false, createdAt: 1, updatedAt: 2 },
];

const mocks = vi.hoisted(() => ({
  preview: vi.fn(async () => preview),
  facts: vi.fn(async () => facts),
  create: vi.fn(async (_c: string, scope: string, fact: string) => ({ id: 'm9', scope, fact, origin: 'user', pinned: false, excluded: false, stale: false, createdAt: 3, updatedAt: 3 }) as MemoryFact),
  update: vi.fn(async () => facts[0]),
  del: vi.fn(async () => {}),
  pin: vi.fn(async () => {}),
  exclude: vi.fn(async () => {}),
  retain: vi.fn(async () => true),
  setRetain: vi.fn(async () => {}),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => String(e),
  api: {
    contextPreview: mocks.preview,
    memoryFacts: mocks.facts,
    memoryCreate: mocks.create,
    memoryUpdate: mocks.update,
    memoryDelete: mocks.del,
    memorySetPinned: mocks.pin,
    memorySetExcluded: mocks.exclude,
    memoryAutoRetain: mocks.retain,
    memorySetAutoRetain: mocks.setRetain,
  },
}));

describe('ContextPanel', () => {
  it('renders the preview items with kinds, reasons, and token counts', async () => {
    render(<ContextPanel conversation="c1" />);
    await screen.findByText('AGENTS.md');
    expect(screen.getByText('Repository map · 120 files')).toBeInTheDocument();
    expect(screen.getByText('Included next turn · ~948 tokens')).toBeInTheDocument();
    expect(screen.getByText('Project rules')).toBeInTheDocument();
    expect(screen.getByText('user fact in project:p1')).toBeInTheDocument();
    expect(screen.getAllByText(/~\d+ tok/).length).toBeGreaterThan(0);
  });

  it('lists memories with scope, origin, stale, excluded, and reason', async () => {
    const { container } = render(<ContextPanel conversation="c1" />);
    const factsList = await waitFor(() => container.querySelector('.context-facts') as HTMLElement);
    await within(factsList).findByText('use pnpm not npm');
    expect(screen.getByText('CI uses pnpm')).toBeInTheDocument();
    expect(screen.getByText('stale')).toBeInTheDocument();
    expect(screen.getByText('excluded')).toBeInTheDocument();
    expect(screen.getByText('pinned')).toBeInTheDocument();
    expect(screen.getAllByText('agent').length).toBeGreaterThan(0);
  });

  it('creates, pins, excludes, edits, and deletes facts', async () => {
    const { container } = render(<ContextPanel conversation="c1" />);
    const factsList = await waitFor(() => container.querySelector('.context-facts') as HTMLElement);
    await within(factsList).findByText('use pnpm not npm');
    fireEvent.change(screen.getByLabelText('New memory'), { target: { value: 'remember vitest' } });
    fireEvent.click(screen.getByRole('button', { name: /add/i }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith('c1', 'project', 'remember vitest'));

    fireEvent.click(screen.getAllByRole('button', { name: 'Pin memory' })[0]);
    await waitFor(() => expect(mocks.pin).toHaveBeenCalledWith('c1', expect.any(String), true));

    fireEvent.click(screen.getAllByRole('button', { name: 'Exclude memory' })[0]);
    await waitFor(() => expect(mocks.exclude).toHaveBeenCalledWith('c1', expect.any(String), true));

    fireEvent.click(screen.getAllByRole('button', { name: 'Edit memory' })[0]);
    fireEvent.change(screen.getByLabelText('Edit memory text'), { target: { value: 'use pnpm always' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(mocks.update).toHaveBeenCalledWith('c1', 'm1', 'use pnpm always'));

    fireEvent.click(screen.getAllByRole('button', { name: 'Delete memory' })[0]);
    await waitFor(() => expect(mocks.del).toHaveBeenCalledWith('c1', 'm1'));
  });

  it('toggles auto-retain', async () => {
    render(<ContextPanel conversation="c1" />);
    const toggle = await screen.findByRole('checkbox', { name: /store memories/i });
    await waitFor(() => expect(toggle).toBeChecked());
    fireEvent.click(toggle);
    await waitFor(() => expect(mocks.setRetain).toHaveBeenCalledWith(false));
  });

  it('shows an empty state without a conversation', () => {
    render(<ContextPanel conversation={null} />);
    expect(screen.getByText(/open a conversation/i)).toBeInTheDocument();
  });
});
