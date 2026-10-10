import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useConversationTodos } from './PlanChecklist';
import type { TodoItem } from '../lib/types';

const getTodos = vi.hoisted(() => vi.fn());
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: { getTodos } }));
afterEach(() => { getTodos.mockReset(); vi.useRealTimers(); });
const todo: TodoItem = { text: 'Work on project A', status: 'in_progress', updatedAt: 1 };

it('never shows a previous conversation checklist while the next one loads', async () => {
  let resolveNext!: (items: TodoItem[]) => void;
  getTodos.mockResolvedValueOnce([todo]).mockImplementationOnce(() => new Promise<TodoItem[]>(resolve => { resolveNext = resolve; }));
  const view = renderHook(({ id }) => useConversationTodos(id, false), { initialProps: { id: 'a' } });
  await waitFor(() => expect(view.result.current).toEqual([todo]));
  view.rerender({ id: 'b' });
  expect(view.result.current).toEqual([]);
  await act(async () => resolveNext([]));
  expect(view.result.current).toEqual([]);
  view.unmount();
});

it('preserves the checklist reference when repeated polls return unchanged todos', async () => {
  vi.useFakeTimers();
  getTodos.mockImplementation(async () => [{ ...todo }]);
  const view = renderHook(() => useConversationTodos('a', true));
  await act(async () => {});
  const initial = view.result.current;
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(getTodos).toHaveBeenCalledTimes(3);
  expect(view.result.current).toBe(initial);
  view.unmount();
});
