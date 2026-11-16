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
  const children = metrics?.children || [];
  const childRounds = children.flatMap(child => child.rounds);
  const childCoverageComplete = children.every(child => child.status !== 'running' && child.rounds.length > 0 && child.rounds.every(round => round.complete));
  const presentationRounds = rounds.filter(round => round.phase === "presentation");
  const modelRounds = rounds.filter(round => round.phase !== 'presentation');
  const firstText = modelRounds.find(round => round.firstVisibleAnswerMs != null);
  const finalAnswer = modelRounds.find(round => round.phase === 'answer');
  const inputTokens = sumMetric(rounds, 'inputTokens');
  const cachedTokens = sumMetric(rounds, 'cachedTokens');
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
      {metrics?.reasoningEffort && <div><dt>Requested reasoning effort</dt><dd>{metrics.reasoningEffort}</dd></div>}
      <div><dt>Prompt tokens</dt><dd>{count(inputTokens)}</dd></div>
      <div><dt>Generated tokens</dt><dd>{count(sumMetric(rounds, 'outputTokens'))}</dd></div>
      <div><dt>Prompt processing</dt><dd>{duration(sumMetric(rounds, 'promptMs'))}</dd></div>
      <div><dt>Backend generation</dt><dd>{duration(sumMetric(rounds, 'generationMs'))}</dd></div>
      <div><dt>Response duration</dt><dd>{duration(metrics?.totalMs)}</dd></div>
      <div><dt>First response text</dt><dd>{duration(firstText?.firstVisibleAnswerMs)}</dd></div>
      <div><dt>Final answer starts</dt><dd>{duration(finalAnswer?.firstVisibleAnswerMs)}</dd></div>
      <div><dt>First streamed token</dt><dd>{duration(rates.firstTokenMs)}</dd></div>
      <div><dt>Reused input tokens</dt><dd>{count(cachedTokens)}</dd></div>
      <div><dt>Uncached input tokens</dt><dd>{count(inputTokens !== null && cachedTokens !== null && cachedTokens <= inputTokens ? inputTokens - cachedTokens : null)}</dd></div>
      <div><dt>Cache writes</dt><dd>{count(sumMetric(rounds, 'cacheWriteTokens'))}</dd></div>
      {presentationRounds.length > 0 && <><div><dt>Visual formatting</dt><dd>{duration(presentationRounds.reduce((sum,round)=>sum+round.durationMs,0))}</dd></div><div><dt>Visual prompt tokens</dt><dd>{count(sumMetric(presentationRounds,"inputTokens"))}</dd></div><div><dt>Visual generated tokens</dt><dd>{count(sumMetric(presentationRounds,"outputTokens"))}</dd></div></>}
      <div><dt>Model rounds</dt><dd>{rounds.length || 'Unavailable'}</dd></div>
      {children.length > 0 && <>
        <div><dt>Delegated model rounds</dt><dd>{childRounds.length}</dd></div>
        <div><dt>Reported delegated input tokens</dt><dd>{count(sumMetric(childRounds, 'inputTokens'))}</dd></div>
        <div><dt>Reported delegated output tokens</dt><dd>{count(sumMetric(childRounds, 'outputTokens'))}</dd></div>
        <div><dt>Reported delegated reused input tokens</dt><dd>{count(sumMetric(childRounds, 'cachedTokens'))}</dd></div>
        <div><dt>Combined input tokens</dt><dd>{count(childCoverageComplete ? sumMetric([...rounds, ...childRounds], 'inputTokens') : null)}</dd></div>
      </>}
    </dl>
    <p>Rates use backend token counts and inference durations. First streamed token measures from model dispatch to the first content, reasoning, or tool token. First response text includes progress messages, and final answer starts measures the accepted answer round. Both start at the native send request and precede rendering. Tool execution can include interactive waits. Response duration includes preparation, inference, tools, and approvals through the text handoff. Visual formatter rounds also contribute to the token totals.</p>
    {message.status !== 'complete' || rounds.some(round => !round.complete) ? <p>This response ended {message.status === 'complete' ? 'with an incomplete model stream' : message.status}. Incomplete timing data is not reported as throughput.</p> : null}
    {!metrics && <p>This message has no recorded measurements.</p>}
    {children.length > 0 && <p>Delegated usage is listed separately from the main response. Missing provider counters remain unavailable. {childCoverageComplete ? 'Recorded child streams are complete.' : 'Child usage is incomplete; running or interrupted work is excluded from a combined total.'}</p>}
  </details>;
}
