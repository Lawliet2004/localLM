import { act, render, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { TerminalPanel, closeTerminalForTab } from './TerminalPanel';

const mocks = vi.hoisted(() => ({
  terminalOpen: vi.fn(),
  terminalAttach: vi.fn(),
  terminalClose: vi.fn(),
  terminalInput: vi.fn(),
  terminalResize: vi.fn(),
  attachHandlers: new Map<string, (e: { type: string; data?: string }) => void>(),
  instances: [] as { write: ReturnType<typeof vi.fn>; dispose: ReturnType<typeof vi.fn> }[],
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    terminalOpen: mocks.terminalOpen,
    terminalAttach: mocks.terminalAttach,
    terminalClose: mocks.terminalClose,
    terminalInput: mocks.terminalInput,
    terminalResize: mocks.terminalResize,
  },
}));

// Only the rendering boundary is mocked; the session map and attach
// lifecycle under test run for real.
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    write = vi.fn();
    open = vi.fn();
    dispose = vi.fn();
    loadAddon = vi.fn();
    onData = vi.fn(() => ({ dispose: vi.fn() }));
    constructor() { mocks.instances.push(this); }
  },
}));
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = vi.fn(); } }));

// jsdom has no ResizeObserver; the panel only needs the shape.
(globalThis as Record<string, unknown>).ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

describe('TerminalPanel', () => {
  let seq = 0;
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.attachHandlers.clear();
    mocks.instances.length = 0;
    seq = 0;
    mocks.terminalOpen.mockImplementation(() => Promise.resolve({ id: `pty-${++seq}` }));
    mocks.terminalAttach.mockImplementation((id: string, onEvent: (e: { type: string; data?: string }) => void) => {
      mocks.attachHandlers.set(id, onEvent);
      return Promise.resolve({ id, exited: false });
    });
    mocks.terminalClose.mockResolvedValue(undefined);
    mocks.terminalInput.mockResolvedValue(undefined);
    mocks.terminalResize.mockResolvedValue(undefined);
  });

  it('attaches the new session when tabKey changes and drops the old channel\'s events', async () => {
    const { rerender } = render(<TerminalPanel tabKey="tab-a" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-1', expect.any(Function)));
    rerender(<TerminalPanel tabKey="tab-b" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-2', expect.any(Function)));
    // A fresh terminal replaced the disposed one.
    expect(mocks.instances).toHaveLength(2);
    expect(mocks.instances[0].dispose).toHaveBeenCalled();
    // An event on the detached channel must not reach either terminal.
    act(() => mocks.attachHandlers.get('pty-1')!({ type: 'output', data: 'stale' }));
    act(() => mocks.attachHandlers.get('pty-2')!({ type: 'output', data: 'fresh' }));
    expect(mocks.instances[0].write).not.toHaveBeenCalled();
    expect(mocks.instances[1].write).toHaveBeenCalledWith('fresh');
    expect(mocks.instances[1].write).not.toHaveBeenCalledWith('stale');
  });

  it('kills a still-pending spawn once it resolves after the tab closed', async () => {
    let resolveOpen!: (v: { id: string }) => void;
    mocks.terminalOpen.mockImplementationOnce(() => new Promise<{ id: string }>(r => { resolveOpen = r; }));
    const view = render(<TerminalPanel tabKey="tab-pending" />);
    await waitFor(() => expect(mocks.terminalOpen).toHaveBeenCalledTimes(1));
    closeTerminalForTab('tab-pending');
    view.unmount();
    await act(async () => { resolveOpen({ id: 'pty-late' }); });
    // The shell that landed after close is still killed, and the cancelled
    // attach never lands on it.
    await waitFor(() => expect(mocks.terminalClose).toHaveBeenCalledWith('pty-late'));
    expect(mocks.terminalAttach).not.toHaveBeenCalled();
    // Reopening the same tab spawns fresh instead of reusing the dead entry.
    render(<TerminalPanel tabKey="tab-pending" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-1', expect.any(Function)));
  });

  it('keeps the session alive across unmount and reattaches on return', async () => {
    const view = render(<TerminalPanel tabKey="tab-keep" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-1', expect.any(Function)));
    view.unmount();
    expect(mocks.terminalClose).not.toHaveBeenCalled();
    render(<TerminalPanel tabKey="tab-keep" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledTimes(2));
    expect(mocks.terminalOpen).toHaveBeenCalledTimes(1);
  });

  it('spawns a fresh shell after the tab is closed', async () => {
    const view = render(<TerminalPanel tabKey="tab-close" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-1', expect.any(Function)));
    view.unmount();
    closeTerminalForTab('tab-close');
    await waitFor(() => expect(mocks.terminalClose).toHaveBeenCalledWith('pty-1'));
    render(<TerminalPanel tabKey="tab-close" />);
    await waitFor(() => expect(mocks.terminalAttach).toHaveBeenCalledWith('pty-2', expect.any(Function)));
  });
});
