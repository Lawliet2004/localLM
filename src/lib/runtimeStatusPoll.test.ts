import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { RuntimeStatus } from './types';

const mock = vi.hoisted(() => ({ runtimeStatus: vi.fn(async (): Promise<RuntimeStatus> => ({ phase: 'ready', message: 'Model ready', modelPath: 'C:/m.gguf' })) }));
vi.mock('./api', () => ({ api: mock, nativeAvailable: true, errorMessage: String }));

const { useRuntimeStatusPoll } = await import('./runtimeStatusPoll');

beforeEach(() => { vi.useFakeTimers(); mock.runtimeStatus.mockClear(); });
afterEach(() => { vi.useRealTimers(); });

it('polls every three seconds while the runtime is ready', async () => {
  const apply = vi.fn();
  const { unmount } = renderHook(({ phase }) => useRuntimeStatusPoll(phase, apply), { initialProps: { phase: 'ready' as const } });
  expect(mock.runtimeStatus).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(3000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(1);
  expect(apply).toHaveBeenCalledWith(expect.objectContaining({ phase: 'ready' }));
  await vi.advanceTimersByTimeAsync(3000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(2);
  unmount();
  await vi.advanceTimersByTimeAsync(9000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(2);
});

it('does not poll while stopped or in error', async () => {
  for (const phase of ['stopped', 'error'] as const) {
    mock.runtimeStatus.mockClear();
    const { unmount } = renderHook(() => useRuntimeStatusPoll(phase, vi.fn()));
    await vi.advanceTimersByTimeAsync(9000);
    expect(mock.runtimeStatus).not.toHaveBeenCalled();
    unmount();
  }
});

it('stops polling when the phase becomes stopped', async () => {
  const { rerender } = renderHook(({ phase }) => useRuntimeStatusPoll(phase, vi.fn()), { initialProps: { phase: 'ready' as RuntimeStatus['phase'] } });
  await vi.advanceTimersByTimeAsync(3000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(1);
  rerender({ phase: 'stopped' });
  await vi.advanceTimersByTimeAsync(9000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(1);
});

it('discards an in-flight status when the runtime is stopped', async () => {
  let resolve!: (status: RuntimeStatus) => void;
  mock.runtimeStatus.mockImplementationOnce(() => new Promise<RuntimeStatus>(done => { resolve = done; }));
  const apply = vi.fn();
  const view = renderHook(({ phase }) => useRuntimeStatusPoll(phase, apply), { initialProps: { phase: 'ready' as RuntimeStatus['phase'] } });
  await vi.advanceTimersByTimeAsync(3000);
  view.rerender({ phase: 'stopped' });
  await act(async () => resolve({ phase: 'ready', message: 'Old response', modelPath: 'C:/old.gguf' }));
  expect(apply).not.toHaveBeenCalled();
  view.unmount();
});

it('waits for a slow status request before starting the next poll', async () => {
  let resolve!: (status: RuntimeStatus) => void;
  mock.runtimeStatus.mockImplementationOnce(() => new Promise<RuntimeStatus>(done => { resolve = done; }));
  const apply = vi.fn();
  const view = renderHook(() => useRuntimeStatusPoll('ready', apply));
  await vi.advanceTimersByTimeAsync(12000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(1);
  await act(async () => resolve({ phase: 'ready', message: 'Ready', modelPath: 'C:/m.gguf' }));
  await vi.advanceTimersByTimeAsync(3000);
  expect(mock.runtimeStatus).toHaveBeenCalledTimes(2);
  view.unmount();
});
