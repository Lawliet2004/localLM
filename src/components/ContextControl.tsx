import { useState } from 'react';
import type { ContextBreakdown, ContextUsage } from '../lib/types';

/* Radius chosen so the circumference is ~100, letting stroke-dashoffset map
   directly onto percentages. */
const RING_RADIUS = 15.9155;

/* Row order mirrors the size the categories usually have: history first,
   frozen prompt material last. */
const BREAKDOWN_ROWS: { key: keyof ContextBreakdown; label: string; className: string }[] = [
  { key: 'messages', label: 'Messages', className: 'messages' },
  { key: 'systemTools', label: 'System tools', className: 'system-tools' },
  { key: 'mcpTools', label: 'MCP tools', className: 'mcp-tools' },
  { key: 'systemPrompt', label: 'System prompt', className: 'system-prompt' },
  { key: 'skills', label: 'Skills', className: 'skills' },
  { key: 'metaContext', label: 'Meta context', className: 'meta-context' },
];

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

/* Compact token counts for the header: 41.5K/1M instead of digit walls.
   Values under 10K keep their full digits. */
function fmtTokens(value: number): string {
  const compact = (scaled: number) => (scaled >= 100 ? Math.round(scaled).toString() : scaled.toFixed(1).replace(/\.0$/, ''));
  if (value >= 1_000_000) return `${compact(value / 1_000_000)}M`;
  if (value >= 10_000) return `${compact(value / 1000)}K`;
  return value.toLocaleString();
}

/* Shares keep one decimal below ten (0.9%) and drop trailing zeros (2%). */
function fmtShare(value: number): string {
  const rounded = Math.round(value * 10) / 10;
  return Number.isInteger(rounded) ? `${rounded}` : rounded.toFixed(1);
}

export function ContextControl({ usage }: { usage?: ContextUsage }) {
  const [pinned, setPinned] = useState(false);
  const used = usage ? usage.inputTokens + (usage.outputTokens ?? 0) : null;
  const exactPercent = used !== null && usage && usage.contextLength > 0
    ? Math.min(100, used / usage.contextLength * 100)
    : null;
  const percent = exactPercent === null ? null : Math.round(exactPercent);
  const level = percent === null ? 'idle' : percent >= 80 ? 'high' : percent >= 55 ? 'mid' : 'low';
  const breakdown = usage?.breakdown;
  const breakdownSum = breakdown
    ? BREAKDOWN_ROWS.reduce((sum, row) => sum + (breakdown[row.key] || 0), 0)
    : 0;
  const hitRate = usage?.avgCacheHitRate ?? usage?.cacheHitRate ?? null;
  const headTotal = usage && used !== null
    ? `${usage.estimated ? '~' : ''}${fmtTokens(used)}/${fmtTokens(usage.contextLength)}${exactPercent === null ? '' : ` (${fmtShare(exactPercent)}%)`}`
    : null;
  return (
    <div className={`context-control context-${level} ${pinned ? 'is-open' : ''}`}>
      <button
        type="button"
        className="context-trigger"
        aria-expanded={pinned}
        aria-haspopup="dialog"
        aria-label={percent === null ? 'Context usage details' : `Context ${percent}% used — hover or click for details`}
        onClick={() => setPinned(p => !p)}
      >
        <ContextRing percent={percent} size={17} stroke={3.4} />
      </button>
      <div className="context-popover" role="dialog" aria-label="Context window usage">
        <div className="context-head">
          <strong>Context windows</strong>
          <small>{headTotal ?? 'No usage measured yet'}</small>
        </div>
        <div className="context-bar" aria-hidden="true">
          <span style={{ width: `${exactPercent ?? 0}%` }} />
        </div>
        {usage && (
          <>
            {breakdown && breakdownSum > 0 && (
              <ul className="context-rows">
                {BREAKDOWN_ROWS.filter(row => (breakdown[row.key] || 0) > 0).map(row => (
                  <li key={row.key} className="context-row">
                    <span className={`context-dot dot-${row.className}`} aria-hidden="true" />
                    <span className="context-label">{row.label}</span>
                    <span className="context-share">{fmtShare((breakdown[row.key] || 0) / breakdownSum * 100)}%</span>
                  </li>
                ))}
              </ul>
            )}
            <div className="context-footer">
              <div className="context-row">
                <span className="context-label">Input</span>
                <span className="context-share">{usage.inputTokens.toLocaleString()}</span>
              </div>
              <div className="context-row">
                <span className="context-label">Output</span>
                <span className="context-share">{usage.outputTokens != null ? usage.outputTokens.toLocaleString() : '—'}</span>
              </div>
              <div className="context-row">
                <span className="context-label">Average cache hit rate</span>
                <span className="context-share">{hitRate != null ? `${fmtShare(hitRate * 100)}%` : '—'}</span>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
