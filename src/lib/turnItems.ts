import type { SessionEvent } from './types';

export type TurnItemKind = 'reasoning' | 'message' | 'tool' | 'plan' | 'approval' | 'subagent' | 'notice' | 'error' | 'compaction';
export type TurnItemStatus = 'running' | 'completed' | 'failed' | 'interrupted' | 'awaiting';
export interface TurnItem {
  id: string;
  runId: string;
  round: number;
  kind: TurnItemKind;
  status: TurnItemStatus;
  startedAt: number;
  endedAt?: number;
  durationMs?: number;
  parentItemId?: string;
  payload: Record<string, unknown>;
  seq: number;
}

const kinds = new Set<TurnItemKind>(['reasoning', 'message', 'tool', 'plan', 'approval', 'subagent', 'notice', 'error', 'compaction']);
const statuses = new Set<TurnItemStatus>(['running', 'completed', 'failed', 'interrupted', 'awaiting']);
const terminalStatuses = new Set<TurnItemStatus>(['completed', 'failed', 'interrupted']);
const record = (value: unknown): Record<string, unknown> | null => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value) && value >= 0;

/** Reduce the same journal events delivered live or read on replay. Sorting
 * and event IDs make retries and out-of-order delivery deterministic. */
export function reduceTurnItems(events: readonly SessionEvent[]): TurnItem[] {
  const items = new Map<string, TurnItem>();
  const textChunks = new Map<string, string[]>();
  const seen = new Set<string>();
  const alreadyOrdered = events.every((event, index) => index === 0 || events[index - 1].seq < event.seq ||
    events[index - 1].seq === event.seq && events[index - 1].id.localeCompare(event.id) <= 0);
  const ordered = alreadyOrdered ? events : [...events].sort((a, b) => a.seq - b.seq || a.id.localeCompare(b.id));
  for (const event of ordered) {
    if (!event.eventType.startsWith('item.') || seen.has(event.id)) continue;
    seen.add(event.id);
    const payload = record(event.payload);
    if (!payload) continue;
    // Forked journals can outlive the original run row; SQLite then clears
    // the outer FK while the immutable item retains its original run ID.
    const outerRunId = event.runId ?? undefined;
    if (event.eventType === 'item.started') {
      const value = record(payload.item);
      if (!value || typeof value.id !== 'string' || !value.id || typeof value.runId !== 'string' || !value.runId ||
          (outerRunId !== undefined && value.runId !== outerRunId) ||
          !finite(value.round) || !Number.isInteger(value.round) || !finite(value.startedAt) ||
          !kinds.has(value.kind as TurnItemKind) || !statuses.has(value.status as TurnItemStatus) || items.has(value.id)) continue;
      items.set(value.id, { id: value.id, runId: value.runId, round: value.round, kind: value.kind as TurnItemKind,
        status: value.status as TurnItemStatus, startedAt: value.startedAt, seq: event.seq,
        parentItemId: typeof value.parentItemId === 'string' ? value.parentItemId : undefined,
        payload: { ...(record(value.payload) ?? {}) } });
      const initialText = record(value.payload)?.text;
      if (typeof initialText === 'string') textChunks.set(value.id, [initialText]);
      continue;
    }
    if (typeof payload.itemId !== 'string') continue;
    const current = items.get(payload.itemId);
    if (!current || (outerRunId !== undefined && current.runId !== outerRunId) || terminalStatuses.has(current.status)) continue;
    if (event.eventType === 'item.delta') {
      const delta = record(payload.delta);
      if (typeof delta?.text === 'string') {
        const chunks = textChunks.get(current.id) ?? [];
        chunks.push(delta.text); textChunks.set(current.id, chunks);
      }
    } else if (event.eventType === 'item.updated') {
      const update = record(payload.payload);
      if (update) {
        current.payload = { ...current.payload, ...update };
        if (typeof update.text === 'string') textChunks.set(current.id, [update.text]);
      }
      if (statuses.has(payload.status as TurnItemStatus) && !terminalStatuses.has(payload.status as TurnItemStatus)) current.status = payload.status as TurnItemStatus;
    } else if (event.eventType === 'item.completed' && terminalStatuses.has(payload.status as TurnItemStatus)) {
      current.status = payload.status as TurnItemStatus;
      if (finite(payload.endedAt)) current.endedAt = payload.endedAt;
      if (finite(payload.durationMs)) current.durationMs = payload.durationMs;
    }
  }
  for (const [id, chunks] of textChunks) items.get(id)!.payload.text = chunks.join('');
  return [...items.values()].sort((a, b) => a.seq - b.seq);
}

const listeners = new Map<string, Set<(event: SessionEvent) => void>>();
export function publishTurnItem(event: SessionEvent) {
  if (!event.eventType.startsWith('item.')) return;
  for (const listener of listeners.get(event.conversationId) ?? []) listener(event);
}
export function subscribeTurnItems(conversationId: string, listener: (event: SessionEvent) => void) {
  const group = listeners.get(conversationId) ?? new Set();
  group.add(listener); listeners.set(conversationId, group);
  return () => { group.delete(listener); if (!group.size) listeners.delete(conversationId); };
}
