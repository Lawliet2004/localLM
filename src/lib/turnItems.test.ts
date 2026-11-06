import { expect, it, vi } from 'vitest';
import { publishTurnItem, reduceTurnItems, subscribeTurnItems } from './turnItems';
import type { SessionEvent } from './types';

const event = (seq: number, eventType: string, payload: unknown, runId = 'run'): SessionEvent => ({ id: `event-${seq}`, conversationId: 'chat', runId, seq, eventType, payload, createdAt: seq, ignorable: false });
const start = event(1, 'item.started', { item: { id: 'thought', runId: 'run', round: 0, kind: 'reasoning', status: 'running', startedAt: 100, payload: {} } });
const first = event(2, 'item.delta', { itemId: 'thought', delta: { text: 'First ' } });
const second = event(3, 'item.delta', { itemId: 'thought', delta: { text: 'thought' } });
const done = event(4, 'item.completed', { itemId: 'thought', status: 'completed', endedAt: 125, durationMs: 25 });

it('replays the same item from duplicate and out-of-order live events', () => {
  const replay = reduceTurnItems([start, first, second, done]);
  expect(reduceTurnItems([done, second, first, start, first])).toEqual(replay);
  expect(replay).toEqual([expect.objectContaining({ id: 'thought', status: 'completed', startedAt: 100, endedAt: 125, durationMs: 25, payload: { text: 'First thought' } })]);
});

it('does not mix runs, append after completion, or invent durations', () => {
  const other = event(3, 'item.delta', { itemId: 'thought', delta: { text: 'wrong' } }, 'other');
  const interrupted = event(4, 'item.completed', { itemId: 'thought', status: 'interrupted', endedAt: 500 });
  const later = event(5, 'item.delta', { itemId: 'thought', delta: { text: 'late' } });
  const [item] = reduceTurnItems([later, interrupted, other, first, start]);
  expect(item.payload.text).toBe('First ');
  expect(item.status).toBe('interrupted');
  expect(item.durationMs).toBeUndefined();
});

it('ignores a nonterminal completion and continues accepting deltas', () => {
  const invalid = event(3, 'item.completed', { itemId: 'thought', status: 'running', endedAt: 110 });
  const later = event(4, 'item.delta', { itemId: 'thought', delta: { text: 'continued' } });
  const [item] = reduceTurnItems([start, first, invalid, later]);
  expect(item.status).toBe('running');
  expect(item.payload.text).toBe('First continued');
  expect(item.endedAt).toBeUndefined();
});

it('replays forked items after the original run row has been deleted', () => {
  const orphaned = [start, first, second, done].map(entry => ({ ...entry, runId: null }));
  expect(reduceTurnItems(orphaned)).toEqual(reduceTurnItems([start, first, second, done]));
});

it('delivers live items only to subscribers of their own conversation', () => {
  const one = vi.fn(), other = vi.fn();
  const stopOne = subscribeTurnItems('chat', one);
  const stopOther = subscribeTurnItems('other', other);
  publishTurnItem(start);
  expect(one).toHaveBeenCalledExactlyOnceWith(start);
  expect(other).not.toHaveBeenCalled();
  stopOne(); stopOther();
  publishTurnItem(first);
  expect(one).toHaveBeenCalledTimes(1);
});
