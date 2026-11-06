import { useCallback, useRef, useState } from 'react';
import type { ContextUsage, Message, ToolApproval } from '../lib/types';

export interface SessionRun {
  messages: Message[];
  generating: boolean;
  error: string;
  notice: string;
  activity: { state?: string; activity?: string } | null;
  context?: ContextUsage;
  approvals: ToolApproval[];
  unread: boolean;
  planMode: boolean;
}

export const emptySessionRun = (): SessionRun => ({ messages: [], generating: false, error: '', notice: '', activity: null, approvals: [], unread: false, planMode: false });

/** Updates are addressed to the originating conversation, including while it
 * is hidden. The ref is authoritative inside async stream callbacks. */
export function useSessionRuns() {
  const current = useRef<Record<string, SessionRun>>({});
  const [runs, setRuns] = useState(current.current);
  const get = useCallback((id: string) => current.current[id] ?? emptySessionRun(), []);
  const update = useCallback((id: string, change: Partial<SessionRun> | ((run: SessionRun) => SessionRun)) => {
    const previous = current.current[id] ?? emptySessionRun();
    const next = typeof change === 'function' ? change(previous) : { ...previous, ...change };
    current.current = { ...current.current, [id]: next };
    setRuns(current.current);
  }, []);
  const move = useCallback((from: string, to: string) => {
    const next = { ...current.current, [to]: current.current[from] ?? emptySessionRun() };
    delete next[from]; current.current = next; setRuns(next);
  }, []);
  const forget = useCallback((id: string) => {
    if (current.current[id]?.generating) return;
    const next = { ...current.current }; delete next[id]; current.current = next; setRuns(next);
  }, []);
  return { runs, get, update, move, forget };
}
