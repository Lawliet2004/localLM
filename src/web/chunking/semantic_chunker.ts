/**
 * Semantic Chunker: Chunks documents along natural boundaries (headings, paragraphs,
 * lists, sentences) preserving heading hierarchy and provenance.
 */

import type { RetrievedDocument, EvidenceChunk } from '../types';
import { defaultTokenCounter } from './tokenizer';

export interface ChunkerOptions {
  targetTokens?: number;
  overlapTokens?: number;
}

export function chunkDocument(
  doc: RetrievedDocument,
  options: ChunkerOptions = {}
): EvidenceChunk[] {
  const targetTokens = options.targetTokens ?? 600;
  const overlapTokens = options.overlapTokens ?? 80;

  if (!doc.text || doc.text.trim().length === 0) {
    return [];
  }

  const rawLines = doc.text.split('\n');
  const sections: Array<{ headingPath: string[]; content: string }> = [];

  let currentHeadings: string[] = [];
  let currentBlock: string[] = [];

  const flushBlock = () => {
    if (currentBlock.length > 0) {
      const text = currentBlock.join('\n').trim();
      if (text.length > 0) {
        sections.push({
          headingPath: [...currentHeadings],
          content: text,
        });
      }
      currentBlock = [];
    }
  };

  // 1. Split text into sections based on Markdown-style headings
  for (const line of rawLines) {
    const trimmed = line.trim();
    const headingMatch = trimmed.match(/^(#{1,6})\s+(.*)$/);

    if (headingMatch) {
      flushBlock();
      const level = headingMatch[1].length;
      const headingText = headingMatch[2].trim();

      // Adjust heading hierarchy
      currentHeadings = currentHeadings.slice(0, level - 1);
      currentHeadings.push(headingText);
    } else {
      currentBlock.push(line);
    }
  }
  flushBlock();

  // If no headings found, treat entire document as one section
  if (sections.length === 0 && doc.text.trim().length > 0) {
    sections.push({
      headingPath: [],
      content: doc.text.trim(),
    });
  }

  // 2. Accumulate sections into target-sized chunks with overlap
  const chunks: EvidenceChunk[] = [];
  let chunkIndex = 1;

  for (const section of sections) {
    // Split section content into paragraphs
    const paragraphs = section.content.split(/\n\s*\n/).filter((p) => p.trim().length > 0);

    let accumulatedText = '';
    let accumulatedTokens = 0;
    let headingPrefix = section.headingPath.length > 0 ? `[Section: ${section.headingPath.join(' > ')}]\n` : '';

    for (let pIndex = 0; pIndex < paragraphs.length; pIndex++) {
      const p = paragraphs[pIndex].trim();
      const pTokens = defaultTokenCounter.count(p);

      // If a single paragraph exceeds target tokens, break into sentences
      if (pTokens > targetTokens) {
        const sentences = p.match(/[^.!?]+[.!?]+(\s+|$)|[^.!?]+$/g) || [p];
        for (const sentence of sentences) {
          const sTokens = defaultTokenCounter.count(sentence);
          if (accumulatedTokens + sTokens > targetTokens && accumulatedTokens > 0) {
            // Emit chunk
            const fullText = `${headingPrefix}${accumulatedText}`.trim();
            chunks.push({
              id: `${doc.id}:C${chunkIndex++}`,
              documentId: doc.id,
              url: doc.url,
              title: doc.title,
              headingPath: section.headingPath,
              text: fullText,
              tokenCount: defaultTokenCounter.count(fullText),
              publishedAt: doc.publishedAt,
            });

            // Start new chunk with trailing overlap
            accumulatedText = sentence;
            accumulatedTokens = sTokens;
          } else {
            accumulatedText += (accumulatedText ? ' ' : '') + sentence;
            accumulatedTokens += sTokens;
          }
        }
      } else if (accumulatedTokens + pTokens > targetTokens && accumulatedTokens > 0) {
        // Emit chunk
        const fullText = `${headingPrefix}${accumulatedText}`.trim();
        chunks.push({
          id: `${doc.id}:C${chunkIndex++}`,
          documentId: doc.id,
          url: doc.url,
          title: doc.title,
          headingPath: section.headingPath,
          text: fullText,
          tokenCount: defaultTokenCounter.count(fullText),
          publishedAt: doc.publishedAt,
        });

        // Compute overlap from the end of the current accumulated text
        const words = accumulatedText.split(/\s+/);
        const overlapWords = words.slice(-Math.min(words.length, Math.floor(overlapTokens * 0.75)));
        const overlapText = overlapWords.join(' ');

        accumulatedText = overlapText ? `${overlapText}\n\n${p}` : p;
        accumulatedTokens = defaultTokenCounter.count(accumulatedText);
      } else {
        accumulatedText += (accumulatedText ? '\n\n' : '') + p;
        accumulatedTokens += pTokens;
      }
    }

    // Emit remaining accumulated text for this section
    if (accumulatedText.trim().length > 0) {
      const fullText = `${headingPrefix}${accumulatedText}`.trim();
      chunks.push({
        id: `${doc.id}:C${chunkIndex++}`,
        documentId: doc.id,
        url: doc.url,
        title: doc.title,
        headingPath: section.headingPath,
        text: fullText,
        tokenCount: defaultTokenCounter.count(fullText),
        publishedAt: doc.publishedAt,
      });
    }
  }

  return chunks;
}
