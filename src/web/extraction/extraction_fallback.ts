/**
 * Extraction Fallback: Robust fallback routines when main HTML parsing produces
 * insufficient text or encounters malformed DOM structures.
 */

import type { ExtractionResult } from './main_content';
import { computeContentHash } from './main_content';

export function fallbackSnippetExtraction(
  title: string,
  snippet: string,
  _url?: string
): ExtractionResult {
  const text = `${title}\n\n${snippet}`.trim();
  return {
    title,
    text,
    contentHash: computeContentHash(text),
    characters: text.length,
    confidence: 0.45,
    method: 'plain_text',
    headings: [],
    links: [],
    tables: [],
  };
}

export function ensureSufficientExtraction(
  result: ExtractionResult,
  fallbackSnippet?: string,
  fallbackTitle?: string
): ExtractionResult {
  if (result.characters >= 150) {
    return result;
  }

  if (fallbackSnippet && fallbackSnippet.trim().length > 0) {
    return fallbackSnippetExtraction(
      result.title || fallbackTitle || 'Web Result',
      fallbackSnippet,
      result.canonicalUrl || ''
    );
  }

  return result;
}
