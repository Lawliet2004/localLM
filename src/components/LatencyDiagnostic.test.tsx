import { expect, it } from 'vitest';
import { diagnosticRecords, percentile } from './LatencyDiagnostic';
it('keeps failed fixture results distinct from valid timings and rejects invented/nonfinite samples', () => {
  const record = { task: 'research', mode: 'quick', tools: true, cache: 'cold', correct: false, complete: true, firstAnswerMs: 500, totalMs: 1000, answer: 'Uncited answer' };
  expect(diagnosticRecords({ schemaVersion: 1, records: [record] })[0].correct).toBe(false);
  expect(() => diagnosticRecords({ schemaVersion: 1, records: [{ ...record, firstAnswerMs: -1 }] })).toThrow();
  expect(percentile([30, 10, 20], .5)).toBe(20);
  expect(percentile([30, 10, 20], .95)).toBe(30);
  expect(percentile([], .5)).toBeNull();
  expect(percentile([10, 20, 30, 40], .5)).toBe(25);
});
