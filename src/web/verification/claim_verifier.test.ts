import { describe, it, expect } from 'vitest';
import { ClaimVerifier } from './claim_verifier';
import type { EvidenceClaim } from '../types';

describe('Claim Verifier (Section 53-55)', () => {
  const evidence: EvidenceClaim[] = [
    {
      id: 'CLM-1',
      claim: 'Model X contains 8B parameters and supports 128K context window.',
      supportingSources: ['S1'],
      status: 'supported',
      confidence: 0.95,
    },
    {
      id: 'CLM-2',
      claim: 'React 19 was released on December 5, 2024.',
      supportingSources: ['S2'],
      status: 'supported',
      confidence: 0.98,
    },
  ];

  it('correctly classifies supported claims with matching facts and numbers', () => {
    const verifier = new ClaimVerifier();
    const claims = [
      'Model X has 8B parameters.',
      'React 19 was released on December 5, 2024.',
    ];

    const report = verifier.verifyClaimsDeterministic(claims, evidence);
    expect(report.allSupported).toBe(true);
    expect(report.supportedCount).toBe(2);
    expect(report.unsupportedCount).toBe(0);
    expect(report.claims[0].status).toBe('SUPPORTED');
    expect(report.claims[1].status).toBe('SUPPORTED');
  });

  it('detects hallucinated numbers and flags them as UNSUPPORTED', () => {
    const verifier = new ClaimVerifier();
    // Claim states 9B parameters instead of 8B in evidence
    const claims = ['Model X contains 9B parameters.'];

    const report = verifier.verifyClaimsDeterministic(claims, evidence);
    expect(report.allSupported).toBe(false);
    expect(report.unsupportedCount).toBe(1);
    expect(report.claims[0].status).toBe('UNSUPPORTED');
  });

  it('flags unmentioned / fabricated claims as UNSUPPORTED', () => {
    const verifier = new ClaimVerifier();
    const claims = ['The framework was written in Haskell by aliens in 1980.'];

    const report = verifier.verifyClaimsDeterministic(claims, evidence);
    expect(report.allSupported).toBe(false);
    expect(report.unsupportedCount).toBe(1);
    expect(report.claims[0].status).toBe('UNSUPPORTED');
  });
});
