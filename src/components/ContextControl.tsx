import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ContextUsage } from '../lib/types';

export function ContextControl({ conversationId, usage, busy }: { conversationId: string; usage?: ContextUsage; busy: boolean }) {
  const [auto, setAuto] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let disposed = false;
    setAuto(true); setError('');
    if (nativeAvailable && conversationId !== 'new') {
      api.compactionStatus(conversationId).then(s => { if (!disposed) setAuto(s.auto); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    }
    return () => { disposed = true; };
  }, [conversationId]);
  const percent = usage && usage.contextLength > 0 ? Math.min(100, Math.round((usage.inputTokens + usage.responseReserve) / usage.contextLength * 100)) : null;
  return <details className="context-control">
    <summary title="Context usage and automatic compaction"><span className="context-ring" style={{ '--used': `${percent ?? 0}%` } as React.CSSProperties} />{percent === null ? 'Context' : `${percent}%`}</summary>
    <div className="context-popover">
      <strong>Context window</strong>
      {usage && <p>{usage.estimated ? 'Estimated: ' : ''}{usage.inputTokens.toLocaleString()} input + {usage.responseReserve.toLocaleString()} reserved / {usage.contextLength.toLocaleString()} tokens</p>}
      <label><input type="checkbox" checked={auto} disabled={busy || saving || conversationId === 'new' || !nativeAvailable} onChange={async e => {
        const next = e.target.checked; setSaving(true); setError('');
        try { await api.setCompactionAuto(conversationId, next); setAuto(next); } catch (e) { setError(errorMessage(e)); } finally { setSaving(false); }
      }} /> Auto-compact at 80%</label>
      <p>Older activity is archived before the next model round. The current request and latest tool results stay in context, and generation continues.</p>
      {conversationId === 'new' && <small>Enabled by default for new tasks.</small>}
      {error && <p role="alert">{error}</p>}
    </div>
  </details>;
}
