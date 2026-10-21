import type { Message } from '../lib/types';
import { responseRates, sumMetric } from '../lib/responseMetrics';

const speed = (value: number | null) => value === null ? 'Unavailable' : `${value.toFixed(1)} tok/s`;
const duration = (value?: number | null) => typeof value === 'number' && Number.isFinite(value) && value >= 0 ? `${(value / 1000).toFixed(2)}s` : 'Unavailable';
const count = (value: number | null) => value === null ? 'Unavailable' : value.toLocaleString();

export function ResponseMetrics({ message }: { message: Message }) {
  if (message.role !== 'assistant' || message.status === 'streaming') return null;
  const metrics = message.metrics;
  const rates = responseRates(metrics);
  const rounds = metrics?.rounds || [];
  const presentationRounds = rounds.filter(round => round.phase === "presentation");
  const reasoningDurations = rounds.flatMap(round => typeof round.reasoningMs === 'number' && Number.isFinite(round.reasoningMs) && round.reasoningMs >= 0 ? [round.reasoningMs] : []);
  return <details className="response-metrics">
    <summary aria-label="Inference performance details">Response diagnostics</summary>
    <dl>
      <div><dt>Generation rate</dt><dd>{speed(rates.generation)}</dd></div>
      <div><dt>Prompt rate</dt><dd>{speed(rates.prompt)}</dd></div>
      <div><dt>Observed reasoning</dt><dd>{duration(reasoningDurations.length ? reasoningDurations.reduce((sum, value) => sum + value, 0) : null)}</dd></div>
      <div><dt>Preparation</dt><dd>{duration(metrics?.preparationMs)}</dd></div>
      <div><dt>Tool execution</dt><dd>{duration(metrics?.toolsMs)}</dd></div>
      <div><dt>Approval wait</dt><dd>{duration(metrics?.approvalMs)}</dd></div>
      <div><dt>Model</dt><dd>{metrics?.model || 'Unavailable'}</dd></div>
      <div><dt>Prompt tokens</dt><dd>{count(sumMetric(rounds, 'inputTokens'))}</dd></div>
      <div><dt>Generated tokens</dt><dd>{count(sumMetric(rounds, 'outputTokens'))}</dd></div>
      <div><dt>Prompt processing</dt><dd>{duration(sumMetric(rounds, 'promptMs'))}</dd></div>
      <div><dt>Backend generation</dt><dd>{duration(sumMetric(rounds, 'generationMs'))}</dd></div>
      <div><dt>Response duration</dt><dd>{duration(metrics?.totalMs)}</dd></div>
      <div><dt>First visible answer</dt><dd>{duration(rounds.find(round => round.firstVisibleAnswerMs != null)?.firstVisibleAnswerMs)}</dd></div>
      <div><dt>First streamed token</dt><dd>{duration(rates.firstTokenMs)}</dd></div>
      <div><dt>Reused input tokens</dt><dd>{count(sumMetric(rounds, 'cachedTokens'))}</dd></div>
      <div><dt>Cache writes</dt><dd>{count(sumMetric(rounds, 'cacheWriteTokens'))}</dd></div>
      {presentationRounds.length > 0 && <><div><dt>Visual formatting</dt><dd>{duration(presentationRounds.reduce((sum,round)=>sum+round.durationMs,0))}</dd></div><div><dt>Visual prompt tokens</dt><dd>{count(sumMetric(presentationRounds,"inputTokens"))}</dd></div><div><dt>Visual generated tokens</dt><dd>{count(sumMetric(presentationRounds,"outputTokens"))}</dd></div></>}
      <div><dt>Model rounds</dt><dd>{rounds.length || 'Unavailable'}</dd></div>
    </dl>
    <p>GEN and PROMPT use backend token counts and inference durations. TTFS uses a monotonic native timer from dispatch of the first model request to its first content, reasoning, or tool token; it includes HTTP and stream handling, before UI rendering. First visible answer measures from the native send request to the first nonempty answer chunk, excluding reasoning and tool tokens; it precedes UI rendering. Reasoning duration spans observed reasoning chunks only. Preparation covers the initial send-to-first-dispatch interval. Tool time excludes approval-dialog wait and can include interactive tool waits. These phases do not partition total duration; network, prompt processing and round bookkeeping also contribute. Response duration starts at the native send request and includes preparation, tool calls and approvals, ending before final persistence and UI delivery. Multi-round rates are weighted across completed generations. Visual formatter rounds, when present, contribute to these totals and are also listed separately; transport completion does not certify successful rendering.</p>
    {message.status !== 'complete' || rounds.some(round => !round.complete) ? <p>This response ended {message.status === 'complete' ? 'with an incomplete model stream' : message.status}. Incomplete timing data is not reported as throughput.</p> : null}
    {!metrics && <p>This message has no recorded measurements.</p>}
  </details>;
}
