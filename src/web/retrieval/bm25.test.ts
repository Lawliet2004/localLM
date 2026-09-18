import { describe, it, expect } from 'vitest';
import { InMemoryBM25 } from './bm25';
import type { EvidenceChunk } from '../types';

describe('In-Memory BM25', () => {
  const chunks: EvidenceChunk[] = [
    {
      id: 'c1',
      documentId: 'd1',
      url: 'https://example.com/1',
      title: 'Python release',
      text: 'Python 3.13 was released with free-threaded mode and an improved interactive interpreter.',
      tokenCount: 15,
    },
    {
      id: 'c2',
      documentId: 'd2',
      url: 'https://example.com/2',
      title: 'Rust release',
      text: 'Rust 2024 edition features improved async traits and borrow checker ergonomics.',
      tokenCount: 14,
    },
    {
      id: 'c3',
      documentId: 'd3',
      url: 'https://example.com/3',
      title: 'Cooking recipe',
      text: 'Delicious chocolate chip cookies recipe with butter and flour.',
      tokenCount: 12,
    },
  ];

  it('ranks relevant technical keywords highest', () => {
    const bm25 = new InMemoryBM25();
    bm25.buildIndex(chunks);

    const hits = bm25.search('free-threaded Python interpreter');
    expect(hits.length).toBeGreaterThan(0);
    expect(hits[0].chunk.id).toBe('c1');
    expect(hits[0].score).toBeGreaterThan(0);
  });

  it('handles zero-match queries gracefully', () => {
    const bm25 = new InMemoryBM25();
    bm25.buildIndex(chunks);

    const hits = bm25.search('nonexistentquantumgibberish');
    expect(hits).toHaveLength(0);
  });
});
