/**
 * Citation Renderer: Parses citation tags ([S1], [S2]), validates them against the application
 * SourceRegistry, strips hallucinated citations, and transforms them into real, clickable hyperlinks.
 */

import type { SourceRegistry } from '../types';
import { escapeHtml } from '../security/content_sanitizer';

export interface CitationValidationResult {
  cleanedText: string;
  citedSourceIds: string[];
  invalidSourceIds: string[];
  valid: boolean;
}

export function validateAndCleanCitations(
  text: string,
  sources: SourceRegistry
): CitationValidationResult {
  const citedSourceIds: string[] = [];
  const invalidSourceIds: string[] = [];

  // Match [S1], [S2, S3], etc.
  const cleanedText = text.replace(/\[(S\d+(?:\s*,\s*S\d+)*)\]/gi, (_match, group) => {
    const ids = group.split(',').map((s: string) => s.trim().toUpperCase());
    const validGroupIds: string[] = [];

    for (const id of ids) {
      if (sources[id]) {
        if (!citedSourceIds.includes(id)) citedSourceIds.push(id);
        validGroupIds.push(id);
      } else {
        if (!invalidSourceIds.includes(id)) invalidSourceIds.push(id);
      }
    }

    if (validGroupIds.length === 0) {
      return ''; // Drop unsupported/hallucinated citation
    }

    return `[${validGroupIds.join(', ')}]`;
  });

  return {
    cleanedText: cleanedText.replace(/[ \t]{2,}/g, ' '),
    citedSourceIds,
    invalidSourceIds,
    valid: invalidSourceIds.length === 0,
  };
}

/**
 * Transforms [S1] tags into Markdown clickable citations: [Title](url) or [S1](url).
 */
export function renderMarkdownCitations(text: string, sources: SourceRegistry): string {
  return text.replace(/\[(S\d+(?:\s*,\s*S\d+)*)\]/gi, (_match, group) => {
    const ids = group.split(',').map((s: string) => s.trim().toUpperCase());
    const rendered = ids.map((id: string) => {
      const source = sources[id];
      return source ? `[[${id}]](${source.url.replace(/\(/g,'%28').replace(/\)/g,'%29')})` : `[${id}]`;
    });
    return rendered.join(', ');
  });
}

/**
 * Generates formatted bibliography / sources list at the end of the response.
 */
export function renderSourcesSection(citedIds: string[], sources: SourceRegistry): string {
  if (citedIds.length === 0) return '';

  const lines = ['\n\n### Sources:'];
  citedIds.forEach((id) => {
    const s = sources[id];
    if (s) {
      const dateStr = s.publishedAt ? ` — ${s.publishedAt}` : '';
      lines.push(`- **[${id}]** [${escapeHtml(s.title).replace(/[\[\]\\]/g,'\\$&')}](${s.url.replace(/\(/g,'%28').replace(/\)/g,'%29')})${dateStr} (${s.domain})`);
    }
  });

  return lines.join('\n');
}
