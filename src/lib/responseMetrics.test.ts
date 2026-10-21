import { describe, expect, it } from 'vitest';
import { responseRates, sumMetric } from './responseMetrics';
import type { InferenceRound, ResponseMetrics } from './types';

const round: InferenceRound = { complete: true, firstTokenMs: 820, durationMs: 5000,
  usage: { inputTokens: 1085, processedPromptTokens: 385, outputTokens: 123, promptMs: 1000, generationMs: 5000, cachedTokens: 700 } };
const metrics: ResponseMetrics = { model: 'test.gguf', totalMs: 5820, rounds: [round] };
describe('recorded response metrics', () => {
  it('uses backend timing and excludes cached tokens from prompt throughput', () => {
    expect(responseRates(metrics)).toEqual({ generation: 24.6, prompt: 385, firstTokenMs: 820 });
  });
  it('weights multiple rounds by processing time', () => {
    const next = { ...metrics, rounds: [round, { ...round, usage: { ...round.usage, outputTokens: 77, generationMs: 1000 } }] };
    expect(responseRates(next).generation).toBeCloseTo(200 / 6);
    expect(sumMetric(next.rounds, 'cachedTokens')).toBe(1400);
  });
  it('prefers native rates, including llama.cpp first-token timing semantics', () => {
    expect(responseRates({ ...metrics, rounds: [{ ...round, usage: { outputTokens: 2, generationMs: 53.14, generationTokensPerSecond: 18.818 } }] }).generation).toBe(18.818);
  });
  it('preserves unavailable, zero and incomplete data without inventing rates', () => {
    expect(responseRates()).toEqual({ generation: null, prompt: null, firstTokenMs: null });
    for (const value of [undefined, null, NaN, -1, 0]) {
      expect(responseRates({ ...metrics, rounds: [{ ...round, usage: { ...round.usage, generationMs: value } }] }).generation).toBeNull();
    }
    expect(responseRates({ ...metrics, rounds: [{ ...round, complete: false }] }).generation).toBeNull();
    expect(sumMetric([round, { ...round, usage: {} }], 'outputTokens')).toBeNull();
    expect(sumMetric([{ ...round, usage: { cachedTokens: 0 } }], 'cachedTokens')).toBe(0);
  });
});
