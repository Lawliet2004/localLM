import type { InferenceRound, ResponseMetrics } from './types';

type UsageKey = keyof InferenceRound['usage'];
export function sumMetric(rounds: InferenceRound[], key: UsageKey): number | null {
  if (!rounds.length) return null;
  let total = 0;
  for (const round of rounds) {
    const value = round.usage[key];
    if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return null;
    total += value;
  }
  return total;
}
export function responseRates(metrics?: ResponseMetrics | null) {
  const rounds = metrics?.rounds || [];
  function rate(tokens: UsageKey, duration: UsageKey, reported: UsageKey) {
    if (!rounds.every(round => round.complete)) return null;
    if (rounds.length === 1) {
      const native = rounds[0].usage[reported];
      if (typeof native === 'number' && Number.isFinite(native) && native >= 0) return native;
    }
    // llama.cpp's native generation rate may exclude the first generated
    // token from the decode interval. Preserve its reported measurement.
    if (rounds.length && rounds.every(round => typeof round.usage[reported] === 'number' && Number.isFinite(round.usage[reported]) && round.usage[reported]! >= 0 && typeof round.usage[duration] === 'number' && round.usage[duration]! > 0)) {
      const milliseconds = sumMetric(rounds, duration)!;
      return rounds.reduce((sum, round) => sum + round.usage[reported]! * round.usage[duration]!, 0) / milliseconds;
    }
    const count = sumMetric(rounds, tokens), ms = sumMetric(rounds, duration);
    return count !== null && ms !== null && ms > 0 ? count * 1000 / ms : null;
  }
  const first = rounds[0]?.firstTokenMs;
  return {
    generation: rate('outputTokens', 'generationMs', 'generationTokensPerSecond'), prompt: rate('processedPromptTokens', 'promptMs', 'promptTokensPerSecond'),
    firstTokenMs: typeof first === 'number' && Number.isFinite(first) && first >= 0 ? first : null,
  };
}
