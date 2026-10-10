/**
 * Chunk Retrieval: BM25 lexical ranking with a per-document diversity cap.
 */

import type { EvidenceChunk } from '../types';
import { InMemoryBM25 } from './bm25';
import { applyDocumentDiversity } from './diversity';

export function retrieveChunks(
  query: string,
  chunks: EvidenceChunk[],
  options: { candidateLimit?: number; maxChunksPerDoc?: number } = {}
): EvidenceChunk[] {
  if (chunks.length === 0) return [];

  const bm25 = new InMemoryBM25();
  bm25.buildIndex(chunks);
  const ranked = bm25.search(query, chunks.length).map((hit) => hit.chunk);

  const diverse = applyDocumentDiversity(ranked, options.maxChunksPerDoc ?? 2);
  return diverse.slice(0, options.candidateLimit ?? 25);
}
