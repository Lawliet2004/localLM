import { describe, it, expect } from 'vitest';
import { ClaimVerifier, arithmeticRecord, citationSupportsQuote, trustedSourceUrl } from './claim_verifier';
import type { EvidenceClaim, LLMProvider } from '../types';

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
    // "has 8B" is a paraphrase of "contains 8B". Overlap does not certify it.
    expect(report.claims[0].status).not.toBe('SUPPORTED');
    expect(report.claims[0].assessment).toBe('lexical_match');
    expect(report.claims[1].status).toBe('SUPPORTED');
    expect(report.claims[1].assessment).toBe('exact_excerpt');
    const reversed = verifier.verifyClaimsDeterministic(['Alice defeated Bob'], [{
      id: 'CLM-3', claim: 'Bob defeated Alice', supportingSources: ['S3'], status: 'supported', confidence: 1,
    }]);
    expect(reversed.claims[0].status).not.toBe('SUPPORTED');
    const swapped = verifier.verifyClaimsDeterministic(['Model X contains 9B parameters.'], evidence);
    expect(swapped.claims[0].status).not.toBe('SUPPORTED');
    const negated = verifier.verifyClaimsDeterministic(['React 19 was not released on December 5, 2024.'], evidence);
    expect(negated.claims[0].status).not.toBe('SUPPORTED');
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

  it('does not treat reversed roles, swapped units, or stale wording as support', () => {
    const verifier = new ClaimVerifier();
    const passage = (claim: string): EvidenceClaim => ({
      id: 'CLM-U',
      claim,
      supportingSources: ['S1'],
      status: 'supported',
      confidence: 1,
    });

    const reversed = verifier.verifyClaimsDeterministic(
      ['Alice defeated Bob'],
      [passage('Bob defeated Alice')],
    );
    expect(reversed.claims[0].status).toBe('UNSUPPORTED');
    expect(reversed.claims[0].assessment).toBe('lexical_match');

    const distance = verifier.verifyClaimsDeterministic(
      ['The route is 5 km long.'],
      [passage('The route is 5 miles long.')],
    );
    expect(distance.claims[0].status).toBe('UNSUPPORTED');
    expect(distance.claims[0].assessment).not.toBe('exact_excerpt');

    const price = verifier.verifyClaimsDeterministic(
      ['The price is 19.99 dollars.'],
      [passage('The price is 19.99 cents.')],
    );
    expect(price.claims[0].status).toBe('UNSUPPORTED');
    expect(price.claims[0].assessment).not.toBe('exact_excerpt');

    const droppedUnit = verifier.verifyClaimsDeterministic(
      ['The price is 19.99.'],
      [passage('The price is 19.99 cents.')],
    );
    expect(droppedUnit.claims[0].status).toBe('UNSUPPORTED');

    const symbol = verifier.verifyClaimsDeterministic(
      ['The price is $19.99.'],
      [passage('The price is 19.99 cents.')],
    );
    expect(symbol.claims[0].status).toBe('UNSUPPORTED');

    const samePrice = verifier.verifyClaimsDeterministic(
      ['The price is $19.99.'],
      [passage('The price is $19.99.')],
    );
    expect(samePrice.claims[0].status).toBe('SUPPORTED');
    expect(samePrice.claims[0].assessment).toBe('exact_excerpt');

    const stale = verifier.verifyClaimsDeterministic(
      ['The library currently supports 8B parameter models.'],
      [passage('As of 2020, the library supports 8B parameter models.')],
    );
    expect(stale.claims[0].status).toBe('UNSUPPORTED');
    expect(stale.claims[0].assessment).not.toBe('exact_excerpt');

    const currentInPassage = verifier.verifyClaimsDeterministic(
      ['React 19 is the current release.'],
      [passage('React 19 is the current release.')],
    );
    expect(currentInPassage.claims[0].status).toBe('SUPPORTED');
    expect(currentInPassage.claims[0].assessment).toBe('exact_excerpt');
  });

  it('checks that a passage contains the quote without certifying a different answer', () => {
    const passage = 'Bob defeated Alice in straight sets.';
    expect(citationSupportsQuote('Alice defeated Bob', passage)).toBe(false);
    expect(citationSupportsQuote('Bob defeated Alice', passage)).toBe(true);
    expect(citationSupportsQuote('Bob defeated Alice in straight sets.', passage)).toBe(true);

    const answer = new ClaimVerifier().verifyClaimsDeterministic(
      ['Alice defeated Bob'],
      [{ id: 'CLM-Q', claim: passage, supportingSources: ['S1'], status: 'supported', confidence: 1 }],
    );
    expect(answer.claims[0].status).not.toBe('SUPPORTED');
    expect(citationSupportsQuote('Bob defeated Alice', passage)).toBe(true);
  });

  it('rejects a model-written URL that was not retrieved', () => {
    const retrieved = ['https://react.dev/blog/2024/12/05/react-19'];
    expect(trustedSourceUrl(retrieved[0], retrieved)).toBe(true);
    expect(trustedSourceUrl('https://example.com/react-19-current', retrieved)).toBe(false);
    expect(trustedSourceUrl('https://react.dev/blog/2024/12/05/react-19/', retrieved)).toBe(false);
    expect(trustedSourceUrl('https://evil.example/redirect?to=https://react.dev/blog/2024/12/05/react-19', retrieved)).toBe(false);
  });

  it('keeps source values and the locally computed number without certifying the total', () => {
    const sourceValues = [8, 128];
    const record = arithmeticRecord(sourceValues, 136);
    expect(record).toEqual({ sourceValues: [8, 128], computed: 136, kind: 'local_calculation' });
    sourceValues.push(1);
    expect(record.sourceValues).toEqual([8, 128]);

    const report = new ClaimVerifier().verifyClaimsDeterministic(
      ['The total is 136.'],
      [{ id: 'CLM-A', claim: 'The parts are 8 and 128.', supportingSources: ['S1'], status: 'supported', confidence: 1 }],
    );
    expect(report.claims[0].status).toBe('UNSUPPORTED');
  });

  it('labels model-graded support as model_assessed', async () => {
    const claim = 'Model X has about eight billion parameters.';
    const evidence: EvidenceClaim[] = [{
      id: 'CLM-1',
      claim: 'Model X contains 8B parameters and supports 128K context window.',
      supportingSources: ['S1'],
      status: 'supported',
      confidence: 0.9,
    }];
    const llm: LLMProvider = {
      async generate() {
        return {
          text: JSON.stringify({
            claims: [{ claim, status: 'SUPPORTED', sources: ['S1'], assessment: 'exact_excerpt' }],
          }),
        };
      },
    };
    const report = await new ClaimVerifier(llm).verifyClaims([claim], evidence);
    expect(report.claims[0].status).toBe('SUPPORTED');
    expect(report.claims[0].assessment).toBe('model_assessed');
    expect(report.supportedCount).toBe(1);
  });
});
