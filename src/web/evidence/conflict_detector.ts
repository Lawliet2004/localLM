/**
 * Conflict Detector: Detects when retrieved sources disagree or contradict
 * each other (dates, numbers, versions, negative assertions).
 */

import type { EvidenceClaim } from '../types';
import { tokenJaccardSimilarity } from '../ranking/deduplicator';

export function detectEvidenceConflicts(claims: EvidenceClaim[]): EvidenceClaim[] {
  const processed = [...claims];

  for (let i = 0; i < processed.length; i++) {
    for (let j = i + 1; j < processed.length; j++) {
      const c1 = processed[i];
      const c2 = processed[j];

      // If they share subject words
      const overlap = tokenJaccardSimilarity(c1.claim, c2.claim);
      if (overlap >= 0.35) {
        // Extract numbers, units, and versions (e.g. 128K, 256K, 3.13, 2026)
        const numPattern = /\b\d+(?:\.\d+)?(?:k|m|g|b|t)?\b/gi;
        const nums1 = (c1.claim.match(numPattern) || []).map((s) => s.toLowerCase());
        const nums2 = (c2.claim.match(numPattern) || []).map((s) => s.toLowerCase());

        const hasDifferentNumbers =
          nums1.length > 0 &&
          nums2.length > 0 &&
          nums1.some((n) => !nums2.includes(n));

        // Check for polarity contradiction (not, never, doesn't, cannot)
        const neg1 = /\b(not|never|no|doesn't|cannot|unable)\b/i.test(c1.claim);
        const neg2 = /\b(not|never|no|doesn't|cannot|unable)\b/i.test(c2.claim);
        const polarityConflict = (neg1 && !neg2) || (!neg1 && neg2);

        if (hasDifferentNumbers || polarityConflict) {
          c1.status = 'conflicting';
          c2.status = 'conflicting';

          c1.conflictingSources = [
            ...(c1.conflictingSources || []),
            ...c2.supportingSources,
          ];
          c2.conflictingSources = [
            ...(c2.conflictingSources || []),
            ...c1.supportingSources,
          ];

          c1.variants = [c1.claim, c2.claim];
          c2.variants = [c2.claim, c1.claim];
        }
      }
    }
  }

  return processed;
}
