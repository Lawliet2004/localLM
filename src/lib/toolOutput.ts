import { useCallback, useSyncExternalStore } from 'react';

export interface ToolOutput { stdout: string; stderr: string }
const EMPTY: ToolOutput = Object.freeze({ stdout: '', stderr: '' });

/** Transient transport buffers only. The journal/message remains the persisted authority.
 * Output notifications target one tool, at most once per 32ms; lifecycle events
 * still update message metadata immediately. No JSON work on the chunk path. */
export class ToolOutputBuffers {
  private buffers = new Map<string, ToolOutput>();
  private listeners = new Map<string, Set<() => void>>();
  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private key(conversationId: string, callId: string) { return JSON.stringify([conversationId, callId]); }
  read(conversationId: string, callId: string): ToolOutput {
    return this.buffers.get(this.key(conversationId, callId)) ?? EMPTY;
  }
  subscribe(conversationId: string, callId: string, listener: () => void) {
    const key = this.key(conversationId, callId);
    const listeners = this.listeners.get(key) ?? new Set();
    listeners.add(listener); this.listeners.set(key, listeners);
    return () => { listeners.delete(listener); if (!listeners.size) this.listeners.delete(key); };
  }
  append(conversationId: string, callId: string, channel: 'stdout' | 'stderr', chunk: string) {
    if (!chunk) return;
    const key = this.key(conversationId, callId);
    const current = this.buffers.get(key) ?? EMPTY;
    this.buffers.set(key, { ...current, [channel]: current[channel] + chunk });
    if (!this.timers.has(key)) this.timers.set(key, setTimeout(() => {
      this.timers.delete(key);
      this.listeners.get(key)?.forEach(listener => listener());
    }, 32));
  }
  clear(conversationId: string, callId: string) {
    const key = this.key(conversationId, callId);
    clearTimeout(this.timers.get(key)); this.timers.delete(key); this.buffers.delete(key);
    this.listeners.get(key)?.forEach(listener => listener());
  }
}
export const toolOutput = new ToolOutputBuffers();
export function useToolOutput(conversationId: string, callId: string) {
  const subscribe = useCallback((listener: () => void) => toolOutput.subscribe(conversationId, callId, listener), [conversationId, callId]);
  const snapshot = useCallback(() => toolOutput.read(conversationId, callId), [conversationId, callId]);
  return useSyncExternalStore(subscribe, snapshot, () => EMPTY);
}
