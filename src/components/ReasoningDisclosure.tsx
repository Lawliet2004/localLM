import { useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { ActivityIndicator } from './ActivityIndicator';
import { MarkdownView } from './MarkdownView';

export function ReasoningDisclosure({ text, active, durationMs }: { text: string; active: boolean; durationMs?: number }) {
  const [open, setOpen] = useState(false);
  const measured = durationMs !== undefined && Number.isFinite(durationMs) && durationMs >= 0;
  const label = !active && measured ? `Reasoning · ${(durationMs / 1000).toFixed(1)}s` : 'Reasoning';
  return <details className="trace-thought-details" data-testid="turn-reasoning" open={open}
    onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="card-header trace-thought-pill" aria-label={active ? 'Reasoning in progress' : label}>
      <ActivityIndicator active={active} />
      <span className="trace-thought-label">{label}</span>
      <ChevronRight size={14} className="trace-chevron thought-chevron" aria-hidden="true" />
    </summary>
    <div className="trace-thought-content"><MarkdownView content={text} assistant streaming={active} /></div>
  </details>;
}
