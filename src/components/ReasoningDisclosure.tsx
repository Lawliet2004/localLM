import { useEffect, useRef, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { ActivityIndicator } from './ActivityIndicator';
import { MarkdownView } from './MarkdownView';
import { TraceIcon } from './TraceIcon';

export function ReasoningDisclosure({ text, active, durationMs }: { text: string; active: boolean; durationMs?: number }) {
  const [open, setOpen] = useState(false);
  const [inspected, setInspected] = useState(false);
  const inspectedText = useRef(text);
  if (open) inspectedText.current = text;
  const measured = durationMs !== undefined && Number.isFinite(durationMs) && durationMs >= 0;
  // While reasoning streams, show elapsed seconds so a long pause reads as progress.
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    if (!active) return;
    const started = Date.now();
    setElapsedMs(0);
    const timer = setInterval(() => setElapsedMs(Date.now() - started), 1000);
    return () => clearInterval(timer);
  }, [active]);
  const label = active ? `Reasoning · ${Math.floor(elapsedMs / 1000)}s` : measured ? `Reasoning · ${(durationMs / 1000).toFixed(1)}s` : 'Reasoning';
  return <details className="trace-thought-details" data-testid="turn-reasoning" open={open}
    onToggle={event => { setOpen(event.currentTarget.open); if (event.currentTarget.open) setInspected(true); }}>
    <summary onClick={event => { event.preventDefault(); setOpen(value => !value); setInspected(true); }} className="card-header trace-thought-pill" aria-label={active ? 'Reasoning in progress' : label}>
      <ActivityIndicator active={active} />
      <TraceIcon kind="thought" size={13} />
      <span className="trace-thought-label">{label}</span>
      <ChevronRight size={14} className="trace-chevron thought-chevron" aria-hidden="true" />
    </summary>
    {(open || inspected) && <div className="trace-thought-content"><MarkdownView content={open ? text : inspectedText.current} assistant streaming={active && open} /></div>}
  </details>;
}
