import { describe, expect, it } from 'vitest';
import { extractAtomicClaims } from './claim_extractor';
import { ClaimVerifier } from './claim_verifier';

describe('answer claim extraction', () => {
  const release = 'Node.js 24.0.0 was released on May 6, 2025.';

  it('audits factual text without counting citations and source footers as claims', () => {
    const claims = extractAtomicClaims(`${release} [Release notes](https://nodejs.org/en/blog/release/v24.0.0)

Sources consulted:
- [Official Node.js release notes](https://nodejs.org/en/blog/release/v24.0.0)
- Node.js archive: https://nodejs.org/en/blog
`);
    expect(claims).toEqual([release]);
    const report = new ClaimVerifier().verifyClaimsDeterministic(claims, [{
      id: 'release', claim: release, supportingSources: ['S1'], status: 'supported', confidence: 1,
    }]);
    expect(report.allSupported).toBe(true);
  });

  it('preserves linked entity labels while removing reference tags and bare URLs', () => {
    expect(extractAtomicClaims('[Node.js](https://nodejs.org) is a JavaScript runtime. [S1] https://nodejs.org'))
      .toEqual(['Node.js is a JavaScript runtime.']);
    expect(extractAtomicClaims('The current release is version 24 [source](https://nodejs.org).'))
      .toEqual(['The current release is version 24.']);
  });

  it('keeps substantive sections after a supporting-sources footer', () => {
    expect(extractAtomicClaims(`${release}

## Supporting sources
- [Node.js release announcement](https://nodejs.org/en/blog/release/v24.0.0)

## Remaining questions
The operating system supports 64-bit applications.`))
      .toEqual([release, 'The operating system supports 64-bit applications.']);
  });

  it('does not hide factual bullets under a sources heading', () => {
    expect(extractAtomicClaims('Sources:\n- The adapter supplies 60 watts.'))
      .toEqual(['The adapter supplies 60 watts.']);
  });

  it('preserves numeric arrays as factual content', () => {
    expect(extractAtomicClaims('The input vector contains [8, 128].'))
      .toEqual(['The input vector contains [8, 128].']);
  });

  it('retains factual assertions after evidence qualifiers and uncertainty clauses', () => {
    expect(extractAtomicClaims(`Based on current retrieved sources, ${release}
I couldn't verify the shipping date.
I couldn't verify the shipping date, but the adapter supplies 60 watts.
Sources disagree: the adapter supplies 60 watts or 90 watts.`))
      .toEqual([
        release,
        'the adapter supplies 60 watts.',
        'Sources disagree: the adapter supplies 60 watts or 90 watts.',
      ]);
  });
});
