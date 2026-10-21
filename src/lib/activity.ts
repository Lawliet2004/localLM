import type { Message, SessionEvent } from './types';
/** Events arrive in journal sequence. Binary search avoids scanning the whole journal per turn. */
export function partitionActivity(turns: Message[][], events: SessionEvent[]): SessionEvent[][] {
  const partitions: SessionEvent[][] = turns.map(() => []);
  const starts = turns.map(turn => turn.find(message => message.role === 'user')?.createdAt ?? 0);
  for (const event of events) {
    let low = 0, high = starts.length;
    const timestamp = event.createdAt;
    while (low < high) { const middle = (low + high) >>> 1; if (starts[middle] <= timestamp) low = middle + 1; else high = middle; }
    if (low > 0) partitions[low - 1].push(event);
  }
  return partitions;
}
