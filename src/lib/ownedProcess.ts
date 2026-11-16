import { useCallback, useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from './api';
export interface ProcessSnapshot {
  id: string; state: string; exitCode?: number | null; durationMs?: number; error?: string | null;
  output: { cursor: number; stream: string; text: string }[]; nextCursor: number; hasMore: boolean; truncated?: boolean;
}
/** One bounded cursor subscription per visible owned process; no model calls. */
export function useOwnedProcess(conversationId: string, id?: string) {
  const [snapshot, setSnapshot] = useState<ProcessSnapshot | null>(null);
  const [notice, setNotice] = useState<string>();
  const [refresh, setRefresh] = useState(0);
  useEffect(() => {
    setSnapshot(null); setNotice(undefined);
    if (!id || !nativeAvailable || !api.getOwnedProcess) return;
    let disposed = false, cursor = 0;
    let frames: ProcessSnapshot['output'] = [];
    let timer: ReturnType<typeof setTimeout>;
    let incomplete = false;
    async function poll() {
      try {
        const page = await api.getOwnedProcess(conversationId, id!, cursor);
        if (disposed) return;
        frames.push(...page.output);
        let bytes = frames.reduce((sum, frame) => sum + frame.text.length, 0);
        while (bytes > 262144 && frames.length > 1) { bytes -= frames.shift()!.text.length; incomplete = true; }
        cursor = page.nextCursor;
        incomplete ||= page.truncated === true;
        setSnapshot({ ...page, output: [...frames], truncated: incomplete });
        if (page.state === 'running' || page.hasMore) timer = setTimeout(() => void poll(), page.hasMore ? 0 : 750);
      } catch (error) { if (!disposed) setNotice(errorMessage(error)); }
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, [conversationId, id, refresh]);
  const stop = useCallback(async () => {
    if (!id || !api.stopOwnedProcess) return;
    try { await api.stopOwnedProcess(conversationId, id); setRefresh(value => value + 1); }
    catch (error) { setNotice(errorMessage(error)); }
  }, [conversationId, id]);
  return { snapshot: snapshot?.id === id ? snapshot : null, notice, stop };
}
