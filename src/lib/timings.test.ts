import { expect, it } from 'vitest';
import { formatTimings } from './timings';

it('reports measured cache reuse and throughput', () => {
  expect(formatTimings({ round: 0, source: 'llama-server', cacheN: 900, promptN: 100, cacheHitRatio: 0.9, promptPerSecond: 412.4, predictedN: 64, predictedPerSecond: 20.25 }))
    .toBe(`Last round: ${(1000).toLocaleString()} prompt tokens (900 from cache, 90%) · prefill 412 tok/s · 64 generated · decode 20.3 tok/s`);
});

it('omits unreported values instead of estimating them', () => {
  expect(formatTimings({ round: 1, source: 'llama-server', promptN: 12 })).toBe('Last round: 12 prompt tokens evaluated');
  expect(formatTimings({ round: 1, source: 'llama-server' })).toBeNull();
});
