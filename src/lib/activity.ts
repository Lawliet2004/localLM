import type { Message, SessionEvent } from './types';
/** Events arrive in journal sequence. Binary search avoids scanning the whole journal per turn. */
export function partitionActivity(turns: Message[][], events: SessionEvent[]): SessionEvent[][] {
  const partitions: SessionEvent[][] = turns.map(() => []);
  const starts = turns.map(turn => turn.find(message => message.role === 'user')?.createdAt ?? 0);
  const messages = new Map(turns.flatMap((turn, index) => turn.map(message => [message.id, index] as const)));
  const runs = new Map<string, number>();
  for (const event of events) {
    if (!event.runId || !['user_msg', 'turn_end', 'model_round_resumed'].includes(event.eventType)) continue;
    const payload = event.payload as { messageId?: string; failedMessageId?: string } | null;
    const index = messages.get(payload?.messageId ?? payload?.failedMessageId ?? '');
    if (index !== undefined) runs.set(event.runId, index);
  }
  for (const event of events) {
    const explicit = event.runId ? runs.get(event.runId) : undefined;
    if (explicit !== undefined) {
      partitions[explicit].push(event);
      continue;
    }
    let low = 0, high = starts.length;
    const timestamp = event.createdAt;
    while (low < high) { const middle = (low + high) >>> 1; if (starts[middle] <= timestamp) low = middle + 1; else high = middle; }
    if (low > 0) partitions[low - 1].push(event);
  }
  return partitions;
}
