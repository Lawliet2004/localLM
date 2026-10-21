import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { WorkspaceTree } from './EditorTree';

const mocks = vi.hoisted(() => ({ workspaceInspect: vi.fn(), workspaceSearch: vi.fn(), openPath: vi.fn() }));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: { workspaceInspect: mocks.workspaceInspect, workspaceSearch: mocks.workspaceSearch },
}));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: mocks.openPath }));

function tree(overrides: Partial<Parameters<typeof WorkspaceTree>[0]> = {}) {
  return <WorkspaceTree root="C:/w" hidden={false} onOpenFile={() => {}} onChooseWorkspace={() => {}} onError={() => {}} {...overrides} />;
}

describe('EditorTree', () => {
  beforeEach(() => vi.clearAllMocks());

  it('lists entries and reports a clicked file path', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [{ name: 'main.rs', kind: 'file' }], truncated: false });
    const onOpenFile = vi.fn();
    render(tree({ onOpenFile }));
    await user.click(await screen.findByText('main.rs'));
    expect(onOpenFile).toHaveBeenCalledWith('main.rs');
  });

  it('searches the whole workspace and opens a file inside a collapsed directory', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [{ name: 'src', kind: 'directory' }], truncated: false });
    mocks.workspaceSearch.mockResolvedValue({ results: [{ path: 'src/deep/hidden.rs', kind: 'file' }], truncated: false });
    const onOpenFile = vi.fn();
    render(tree({ onOpenFile }));
    await screen.findByText('src');
    const input = screen.getByLabelText('Filter files');
    await user.type(input, 'hidden');
    await vi.waitFor(() => expect(mocks.workspaceSearch).toHaveBeenCalledWith('hidden', 'C:/w'), { timeout: 3000 });
    const hit = await screen.findByText('src/deep/hidden.rs');
    expect(hit.closest('.tree-row')).toHaveAttribute('title', 'src/deep/hidden.rs');
    await user.click(hit);
    expect(onOpenFile).toHaveBeenCalledWith('src/deep/hidden.rs');
    // Clearing restores the lazy tree.
    await user.clear(input);
    expect(await screen.findByText('src')).toBeInTheDocument();
  });

  it('keeps only the newest search results when the query changes mid-flight', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [], truncated: false });
    let resolveFirst: (value: unknown) => void = () => {};
    mocks.workspaceSearch.mockImplementationOnce(() => new Promise(resolve => { resolveFirst = resolve; }));
    mocks.workspaceSearch.mockResolvedValue({ results: [{ path: 'new.rs', kind: 'file' }], truncated: false });
    render(tree());
    const input = await screen.findByLabelText('Filter files');
    await user.type(input, 'ab');
    await vi.waitFor(() => expect(mocks.workspaceSearch).toHaveBeenCalledWith('ab', 'C:/w'), { timeout: 3000 });
    await user.type(input, 'c');
    await vi.waitFor(() => expect(mocks.workspaceSearch).toHaveBeenCalledWith('abc', 'C:/w'), { timeout: 3000 });
    expect(await screen.findByText('new.rs')).toBeInTheDocument();
    await act(async () => { resolveFirst({ results: [{ path: 'old.rs', kind: 'file' }], truncated: false }); });
    expect(screen.queryByText('old.rs')).not.toBeInTheDocument();
    expect(screen.getByText('new.rs')).toBeInTheDocument();
  });

  it('notes when search results are truncated', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [], truncated: false });
    mocks.workspaceSearch.mockResolvedValue({ results: [{ path: 'a.rs', kind: 'file' }], truncated: true });
    render(tree());
    await user.type(await screen.findByLabelText('Filter files'), 'a');
    expect(await screen.findByText(/Showing first 1 matches/i, {}, { timeout: 3000 })).toBeInTheDocument();
  });

  it('drops a delayed search from project A after the tree shows project B', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [{ name: 'b.txt', kind: 'file' }], truncated: false });
    let resolveFirst: (value: unknown) => void = () => {};
    mocks.workspaceSearch.mockImplementationOnce(() => new Promise(resolve => { resolveFirst = resolve; }));
    mocks.workspaceSearch.mockResolvedValue({ results: [{ path: 'b-hit.txt', kind: 'file' }], truncated: false, truncation: null });
    const { rerender } = render(tree({ root: 'C:/a' }));
    const input = await screen.findByLabelText('Filter files');
    await user.type(input, 'hit');
    await vi.waitFor(() => expect(mocks.workspaceSearch).toHaveBeenCalledWith('hit', 'C:/a'), { timeout: 3000 });
    rerender(tree({ root: 'C:/b' }));
    await vi.waitFor(() => expect(mocks.workspaceSearch).toHaveBeenCalledWith('hit', 'C:/b'), { timeout: 3000 });
    expect(await screen.findByText('b-hit.txt')).toBeInTheDocument();
    await act(async () => { resolveFirst({ results: [{ path: 'a-hit.txt', kind: 'file' }], truncated: false }); });
    expect(screen.queryByText('a-hit.txt')).not.toBeInTheDocument();
    expect(screen.getByText('b-hit.txt')).toBeInTheDocument();
  });

  it('does not call a truncated search an authoritative miss', async () => {
    const user = userEvent.setup();
    mocks.workspaceInspect.mockResolvedValue({ entries: [], truncated: false });
    mocks.workspaceSearch.mockResolvedValue({ results: [], truncated: true, truncation: 'depth' });
    render(tree());
    await user.type(await screen.findByLabelText('Filter files'), 'deep');
    expect(await screen.findByText(/stopped at the depth limit/i, {}, { timeout: 3000 })).toBeInTheDocument();
    expect(screen.queryByText('No matching files.')).not.toBeInTheDocument();
  });

  it('ignores a directory expansion that returns after the workspace changes', async () => {
    const user = userEvent.setup();
    let resolveExpand: (value: unknown) => void = () => {};
    mocks.workspaceInspect.mockImplementation((path: string) => {
      if (path === 'src') return new Promise(resolve => { resolveExpand = resolve; });
      return Promise.resolve({ entries: [{ name: 'src', kind: 'directory' }], truncated: false });
    });
    const { rerender } = render(tree({ root: 'C:/a' }));
    await user.click(await screen.findByText('src'));
    mocks.workspaceInspect.mockResolvedValue({ entries: [{ name: 'only-b.txt', kind: 'file' }], truncated: false });
    rerender(tree({ root: 'C:/b' }));
    expect(await screen.findByText('only-b.txt')).toBeInTheDocument();
    await act(async () => { resolveExpand({ entries: [{ name: 'from-a.txt', kind: 'file' }], truncated: false }); });
    expect(screen.queryByText('from-a.txt')).not.toBeInTheDocument();
  });
});
