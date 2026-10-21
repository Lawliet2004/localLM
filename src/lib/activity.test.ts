import { expect, it } from 'vitest';
import { partitionActivity } from './activity';
import type { Message, SessionEvent } from './types';
it('partitions a long journal with one timestamp read per event', () => {
  const turns = Array.from({ length: 1000 }, (_, i) => [{ id: String(i), role: 'user', createdAt: i * 10 } as Message]);
  let reads = 0;
  const events = Array.from({ length: 5000 }, (_, i) => ({ id: String(i), get createdAt() { reads++; return i * 2; } } as SessionEvent));
  const result = partitionActivity(turns, events);
  expect(result).toHaveLength(1000);
  expect(result.every(part => part.length === 5)).toBe(true);
  expect(reads).toBe(5000);
});
