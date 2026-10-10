import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ContextUsage } from '../lib/types';

/* Radius chosen so the circumference is ~100, letting stroke-dashoffset map
   directly onto percentages. */
const RING_RADIUS = 15.9155;

function ContextRing({ percent, size, stroke }: { percent: number | null; size: number; stroke: number }) {
  return (
    <span className="context-ring" style={{ width: size, height: size }} aria-hidden="true">
      <svg viewBox="0 0 36 36" width={size} height={size}>
        <circle className="context-ring-track" cx="18" cy="18" r={RING_RADIUS} fill="none" strokeWidth={stroke} />
        <circle
          className="context-ring-fill"
          cx="18"
          cy="18"
          r={RING_RADIUS}
          fill="none"
          strokeWidth={stroke}
          strokeLinecap="round"
          strokeDasharray="100"
          strokeDashoffset={100 - (percent ?? 0)}
          transform="rotate(-90 18 18)"
        />
      </svg>
      {size > 24 && percent !== null && <span className="context-ring-value">{percent}</span>}
    </span>
  );
}

export function ContextControl({ conversationId, usage, busy, onCompact, compacting }: {
  conversationId: string; usage?: ContextUsage; busy: boolean;
  onCompact?: () => void; compacting?: boolean;
}) {
  const [auto, setAuto] = useState(true);
  const [saving, setSaving] = useState(false);
  const [open, setOpen] = useState(false);
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
  const level = percent === null ? 'idle' : percent >= 80 ? 'high' : percent >= 55 ? 'mid' : 'low';
  return (
    <div className={`context-control context-${level} ${open ? 'is-open' : ''}`}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false); }}>
      <button
        type="button"
        className="context-trigger"
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={percent === null ? 'Context usage details' : `Context ${percent}% used — hover for details`}
        onMouseEnter={() => setOpen(true)}
      >
        <ContextRing percent={percent} size={17} stroke={3.4} />
      </button>
      {open && <div className="context-popover" role="dialog" aria-label="Context usage and compaction">
        <div className="context-popover-head">
          <ContextRing percent={percent} size={46} stroke={4.4} />
          <div className="context-popover-title">
            <strong>Context window</strong>
            <small>
              {usage
                ? `${(usage.inputTokens + usage.responseReserve).toLocaleString()} / ${usage.contextLength.toLocaleString()} tokens`
                : 'No usage measured yet'}
            </small>
          </div>
        </div>
        <dl className="context-metrics" aria-label="Token breakdown">
          <div><dt>Input tokens</dt><dd>{usage ? `${usage.estimated ? '~' : ''}${usage.inputTokens.toLocaleString()}` : '—'}</dd></div>
          <div><dt>Output tokens</dt><dd>{usage?.outputTokens?.toLocaleString() ?? 'Unavailable'}</dd></div>
          <div><dt>Cached input</dt><dd>{usage?.cachedInputTokens?.toLocaleString() ?? 'Unavailable'}</dd></div>
          <div><dt>Reply reserve</dt><dd>{usage?.responseReserve.toLocaleString() ?? '—'}</dd></div>
        </dl>
        {usage && <p className="context-note">Cached input is included in input tokens. Output is from the latest model round. The reply reserve leaves room for the next output.</p>}
        <label className="context-auto"><input type="checkbox" checked={auto} disabled={busy || saving || conversationId === 'new' || !nativeAvailable} onChange={async e => {
          const next = e.target.checked; setSaving(true); setError('');
          try { await api.setCompactionAuto(conversationId, next); setAuto(next); } catch (e) { setError(errorMessage(e)); } finally { setSaving(false); }
        }} /> Auto-compact at 80%</label>
        {onCompact && conversationId !== 'new' && (
          <button
            type="button"
            className="context-compact"
            disabled={busy || compacting}
            onClick={onCompact}
            title="Compact older conversation history into an artifact"
          >
            {compacting ? 'Compacting…' : 'Compact history'}
          </button>
        )}
        <p className="context-note">Older activity is archived before the next model round. The current request and latest tool results stay in context, and generation continues.</p>
        {conversationId === 'new' && <small>Enabled by default for new tasks.</small>}
        {error && <p role="alert">{error}</p>}
      </div>}
    </div>
  );
}
