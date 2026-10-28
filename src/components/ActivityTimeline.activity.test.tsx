import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import type { SessionEvent } from '../lib/types';

const mocks = vi.hoisted(() => ({ getSessionEvents: vi.fn(), onSessionActivity: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: mocks }));
import { useSessionActivity } from './ActivityTimeline';

it('ignores a late journal response after switching conversations and unsubscribes', async () => {
  let resolveOld!: (value: SessionEvent[]) => void;
  const stop = vi.fn();
  const row = (conversationId: string): SessionEvent => ({ id: conversationId, conversationId, seq: 1, eventType: 'tool_call', payload: {}, createdAt: 1, ignorable: false });
  mocks.getSessionEvents.mockImplementation((id: string) => id === 'one' ? new Promise(resolve => { resolveOld = resolve; }) : Promise.resolve([row('two')]));
  mocks.onSessionActivity.mockResolvedValue(stop);
  const view = renderHook(({ id }) => useSessionActivity(id, false), { initialProps: { id: 'one' } });
  await waitFor(() => expect(mocks.getSessionEvents).toHaveBeenCalledWith('one', 0, 500));
  view.rerender({ id: 'two' });
  await waitFor(() => expect(view.result.current.map(event => event.conversationId)).toEqual(['two']));
  await act(async () => { resolveOld([row('one')]); });
  expect(view.result.current.map(event => event.conversationId)).toEqual(['two']);
  expect(stop).toHaveBeenCalledOnce();
  view.unmount();
  expect(stop).toHaveBeenCalledTimes(2);
});
