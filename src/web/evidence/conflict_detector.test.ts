import { describe, it, expect } from 'vitest';
import { detectEvidenceConflicts } from './conflict_detector';
import type { EvidenceClaim } from '../types';

describe('Evidence Conflict Detector', () => {
  it('detects numerical/specification contradictions between sources', () => {
    const claims: EvidenceClaim[] = [
      {
        id: 'c1',
        claim: 'Model X features a 128K token context window.',
        supportingSources: ['S1'],
        status: 'supported',
        confidence: 0.95,
      },
      {
        id: 'c2',
        claim: 'Model X features a 256K token context window.',
        supportingSources: ['S2'],
        status: 'supported',
        confidence: 0.90,
      },
    ];

    const processed = detectEvidenceConflicts(claims);
    expect(processed[0].status).toBe('conflicting');
    expect(processed[1].status).toBe('conflicting');
    expect(processed[0].conflictingSources).toContain('S2');
    expect(processed[1].conflictingSources).toContain('S1');
  });
});
