import { describe, it, expect } from 'vitest';
import { chunkDocument } from './semantic_chunker';
import type { RetrievedDocument } from '../types';

describe('Semantic Chunker', () => {
  it('preserves heading hierarchy and section context', () => {
    const doc: RetrievedDocument = {
      id: 'doc-1',
      url: 'https://example.com/guide',
      domain: 'example.com',
      title: 'Framework Guide',
      retrievedAt: new Date().toISOString(),
      searchResultIds: [],
      text: `# Introduction
This is the intro section with general background.

# Architecture
Here is the architecture overview.

## Components
Detailed explanation of sub-components and interfaces.`,
    };

    const chunks = chunkDocument(doc, { targetTokens: 100, overlapTokens: 20 });
    expect(chunks.length).toBeGreaterThanOrEqual(2);

    const archChunk = chunks.find((c) => c.headingPath?.includes('Architecture'));
    expect(archChunk).toBeDefined();
    expect(archChunk?.documentId).toBe('doc-1');
  });

  it('handles empty or sparse documents without errors', () => {
    const doc: RetrievedDocument = {
      id: 'doc-2',
      url: 'https://example.com/empty',
      domain: 'example.com',
      title: 'Empty',
      retrievedAt: new Date().toISOString(),
      searchResultIds: [],
      text: '',
    };

    const chunks = chunkDocument(doc);
    expect(chunks).toHaveLength(0);
  });
});
