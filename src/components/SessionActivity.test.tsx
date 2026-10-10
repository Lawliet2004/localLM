import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { useSessionActivity } from './ActivityTimeline';
import type { SessionEvent } from '../lib/types';

const fixture = vi.hoisted(() => ({ getSessionEvents: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: fixture }));
const event: SessionEvent = { id: 'event', conversationId: 'chat', seq: 40, eventType: 'model_response', payload: { content: 'Saved answer' }, ignorable: false, createdAt: 1 };

it('continues polling from the saved cursor when generation starts and finishes', async () => {
  fixture.getSessionEvents.mockReset().mockResolvedValueOnce([event]).mockResolvedValue([]);
  const view = renderHook(({ generating }) => useSessionActivity('chat', generating), { initialProps: { generating: false } });
  await waitFor(() => expect(view.result.current).toEqual([event]));
  view.rerender({ generating: true });
  await waitFor(() => expect(fixture.getSessionEvents).toHaveBeenNthCalledWith(2, 'chat', 41, 500));
  view.rerender({ generating: false });
  await waitFor(() => expect(fixture.getSessionEvents).toHaveBeenNthCalledWith(3, 'chat', 41, 500));
  expect(view.result.current).toEqual([event]);
  view.unmount();
});

it('discards a late response from a previously selected conversation', async () => {
  let resolveOld!: (value: SessionEvent[]) => void;
  fixture.getSessionEvents.mockReset().mockImplementationOnce(() => new Promise(resolve => { resolveOld = resolve; })).mockResolvedValue([]);
  const view = renderHook(({ id }) => useSessionActivity(id, false), { initialProps: { id: 'old' } });
  view.rerender({ id: 'new-chat' });
  await act(async () => { resolveOld([event]); });
  expect(view.result.current).toEqual([]);
  expect(fixture.getSessionEvents).toHaveBeenLastCalledWith('new-chat', 0, 500);
  view.unmount();
});
