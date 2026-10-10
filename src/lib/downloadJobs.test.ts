import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { ModelInstallStatus } from './api';

const mock = vi.hoisted(() => ({ downloadJobs: vi.fn(async (): Promise<ModelInstallStatus[]> => []) }));
vi.mock('./api', () => ({ api: mock, nativeAvailable: true, errorMessage: String }));

const job = (over: Partial<ModelInstallStatus>): ModelInstallStatus => ({
  busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null,
  jobId: 0, key: '', label: '', asset: null, assetIndex: 0, assetCount: 0,
  assetReceived: 0, assetTotal: 0, saved: 0, attempt: 0, notice: null, updatedAt: 0,
  ...over,
});

async function load() {
  vi.resetModules();
  return import('./downloadJobs');
}

beforeEach(() => {
  vi.useFakeTimers();
  mock.downloadJobs.mockReset().mockResolvedValue([]);
});
afterEach(() => { vi.useRealTimers(); });

it('keeps the last good job list and marks the store stale when a poll fails', async () => {
  const running = job({ jobId: 3, key: 'model:catalog:a.gguf', label: 'a.gguf', busy: true, phase: 'downloading', received: 10, total: 100 });
  mock.downloadJobs.mockResolvedValueOnce([running]).mockRejectedValue(new Error('offline'));
  const store = await load();
  const { result, unmount } = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(result.current.jobs).toEqual([running]);
  expect(result.current.stale).toBe(false);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(result.current.stale).toBe(true);
  expect(result.current.error).toContain('offline');
  expect(result.current.jobs).toEqual([running]);
  unmount();
});

it('stops polling when the last subscriber leaves', async () => {
  const store = await load();
  const { unmount } = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(mock.downloadJobs).toHaveBeenCalledTimes(1);
  unmount();
  await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
  expect(mock.downloadJobs).toHaveBeenCalledTimes(1);
});

it('computes a transfer rate only for download phases', async () => {
  const running = job({ jobId: 7, key: 'runtime:standard', label: 'runtime', busy: true, phase: 'downloading', asset: 'a.zip', received: 100, total: 1000 });
  mock.downloadJobs
    .mockResolvedValueOnce([running])
    .mockResolvedValueOnce([{ ...running, received: 300 }])
    .mockResolvedValue([{ ...running, phase: 'verifying', received: 300 }]);
  const store = await load();
  const { result, unmount } = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(result.current.rate(7)).toBe(0);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(result.current.rate(7)).toBeCloseTo(200);
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(result.current.rate(7)).toBe(0);
  unmount();
});

it('resets the rate sample when a job moves to a new asset', async () => {
  const running = job({ jobId: 4, key: 'runtime:standard', label: 'llama.cpp', busy: true, phase: 'downloading', asset: 'a.zip', assetIndex: 1, assetCount: 2, received: 100, total: 200 });
  mock.downloadJobs
    .mockResolvedValueOnce([running])
    .mockResolvedValue([{ ...running, asset: 'b.zip', assetIndex: 2, received: 150 }]);
  const store = await load();
  const { result, unmount } = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
  expect(result.current.rate(4)).toBe(0);
  unmount();
});

it('dismissal survives unmount and remount', async () => {
  const failed = job({ jobId: 5, key: 'model:catalog:example.gguf', label: 'example.gguf', phase: 'failed', error: 'stalled' });
  mock.downloadJobs.mockResolvedValue([failed]);
  const store = await load();
  const first = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  act(() => store.dismissJob(failed));
  expect(store.isJobDismissed(failed)).toBe(true);
  first.unmount();
  const second = renderHook(() => store.useDownloadJobs());
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(store.isJobDismissed(failed)).toBe(true);
  second.unmount();
});
