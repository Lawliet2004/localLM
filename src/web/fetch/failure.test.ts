import { describe, expect, it } from 'vitest';
import { FetchFailure, shapeFailedFetch } from './failure';

describe('failed URL reads', () => {
  it('returns status and bounded body as diagnostic-only error data', () => {
    const result = shapeFailedFetch(
      'https://source.example/blocked',
      new FetchFailure('Fetch failed: HTTP 403', 403, 'diagnostic '.repeat(1000)),
    );
    expect(result.isError).toBe(true);
    expect(result.failureKind).toBe('fetch_failed');
    expect(result.status).toBe(403);
    expect((result.diagnosticBody ?? '').length).toBeLessThanOrEqual(4000);
    expect(result.message).toContain('not research evidence');
  });
});
