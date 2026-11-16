import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { SessionEvent } from '../lib/types';
import { useSessionActivity } from './ActivityTimeline';
import { publishTurnItem } from '../lib/turnItems';

const mocks = vi.hoisted(() => ({ load: vi.fn(), subscribe: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: { getSessionEvents: mocks.load, onSessionActivity: mocks.subscribe } }));
const event: SessionEvent = { id: 'call', conversationId: 'one', seq: 42, eventType: 'tool_call', runId: 'run', toolCallId: 'call', payload: { name: 'read_file', arguments: { path: 'app.ts' } }, createdAt: 1, ignorable: false };
beforeEach(() => { vi.useFakeTimers(); mocks.load.mockReset().mockResolvedValue([]); mocks.subscribe.mockReset().mockResolvedValue(vi.fn()); });
afterEach(() => vi.useRealTimers());

it('loads pushed journal activity immediately and ignores an already consumed cursor', async () => {
  let push!: (seq: number) => void;
  const stop = vi.fn();
  mocks.subscribe.mockImplementation((_id: string, callback: (seq: number) => void) => { push = callback; return Promise.resolve(stop); });
  const view = renderHook(() => useSessionActivity('one', false));
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(mocks.subscribe).toHaveBeenCalledWith('one', expect.any(Function));
  mocks.load.mockResolvedValueOnce([event]);
  const before = mocks.load.mock.calls.length;
  await act(async () => { push(42); });
  expect(mocks.load.mock.calls.length).toBe(before + 1);
  expect(view.result.current.events).toEqual([event]);
  await act(async () => { push(42); });
  expect(mocks.load.mock.calls.length).toBe(before + 1);
  view.unmount(); expect(stop).toHaveBeenCalledOnce();
});
it('retains polling recovery when pushed-event subscription fails', async () => {
  mocks.subscribe.mockRejectedValue(new Error('older backend'));
  const view = renderHook(() => useSessionActivity('one', true));
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  mocks.load.mockResolvedValueOnce([event]);
  await act(async () => { await vi.advanceTimersByTimeAsync(750); });
  expect(view.result.current.events).toEqual([event]); view.unmount();
});
it('shows typed live events immediately and merges the committed journal without duplicates', async () => {
  const view = renderHook(() => useSessionActivity('one', true));
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  const item: SessionEvent = { ...event, id: 'item', seq: 42, eventType: 'item.started', payload: { item: { id: 'thought' } } };
  const preceding: SessionEvent = { ...event, id: 'earlier', seq: 41 };
  await act(async () => { publishTurnItem(item); });
  expect(view.result.current.events).toEqual([item]);
  mocks.load.mockResolvedValueOnce([preceding, item]);
  await act(async () => { await vi.advanceTimersByTimeAsync(750); });
  expect(view.result.current.events).toEqual([preceding, item]);
  await act(async () => { publishTurnItem(item); });
  expect(view.result.current.events).toHaveLength(2);
  view.unmount();
});
it('releases a listener that resolves after the conversation has changed', async () => {
  let resolve!: (stop: () => void) => void;
  const stop = vi.fn();
  mocks.subscribe.mockImplementationOnce(() => new Promise<() => void>(done => { resolve = done; }));
  const view = renderHook(() => useSessionActivity('one', false));
  view.unmount();
  await act(async () => { resolve(stop); });
  expect(stop).toHaveBeenCalledOnce();
});

it('loads recent activity first and fetches earlier pages independently of the live cursor', async () => {
  const latest = Array.from({ length: 500 }, (_, index) => ({ ...event, seq: 501 + index }));
  mocks.load.mockResolvedValueOnce(latest);
  const view = renderHook(() => useSessionActivity('one', false));
  await act(async () => { await vi.advanceTimersByTimeAsync(0); });
  expect(mocks.load).toHaveBeenNthCalledWith(1, 'one', null, 500, { latest: true });
  expect(view.result.current.hasEarlier).toBe(true);
  expect(view.result.current.events[0].seq).toBe(501);
  mocks.load.mockResolvedValueOnce([event]);
  await act(async () => { view.result.current.loadEarlier(); });
  expect(mocks.load).toHaveBeenLastCalledWith('one', null, 500, { beforeSeq: 501 });
  expect(view.result.current.events[0].seq).toBe(42);
  expect(view.result.current.events[500].seq).toBe(1000);
  expect(view.result.current.hasEarlier).toBe(false);
  view.unmount();
});
