/**
 * Limit how many passages each document contributes to retrieval.
 */

import type { EvidenceChunk } from '../types';

/**
 * Filters chunks to ensure no single document contributes more than maxChunksPerDocument.
 */
export function applyDocumentDiversity(
  chunks: EvidenceChunk[],
  maxChunksPerDoc: number = 2
): EvidenceChunk[] {
  const docCounts = new Map<string, number>();
  const filtered: EvidenceChunk[] = [];

  for (const chunk of chunks) {
    const count = docCounts.get(chunk.documentId) ?? 0;
    if (count < maxChunksPerDoc) {
      docCounts.set(chunk.documentId, count + 1);
      filtered.push(chunk);
    }
  }

  return filtered;
}
