/**
 * Document Store: full stored document text with stable evidence references.
 *
 * The worker keeps full documents outside model context (SQLite `research`
 * rows) and serves bounded passages with stable `docId:passage` references.
 * Open/find tools read from this store — never only from previously
 * compressed evidence — so facts excluded from the initial summary stay
 * recoverable. PDF text carries page references; scanned PDFs report an
 * explicit OCR-missing failure instead of silent snippet-only evidence.
 */

import type { RetrievedDocument } from '../types';
import { chunkDocument } from '../chunking/semantic_chunker';
import { defaultTokenCounter } from '../chunking/tokenizer';

export interface StoredDocument extends RetrievedDocument {
  fullText: string;
  pages?: string[];
  headings?: string[];
  links?: Array<{ text: string; href: string }>;
  extractionMethod: 'main_content' | 'search_snippet' | 'snippet_salvage' | 'pdf_text' | 'pdf_snippet' | 'github_raw' | 'js_render' | 'wayback' | 'searxng_answer' | 'searxng_infobox';
  ocrRequired?: boolean;
}

export interface Passage {
  ref: string;
  docId: string;
  page?: number;
  section?: string;
  text: string;
  chars: number;
}

const MAX_PASSAGE_CHARS = 2000;
const MAX_PASSAGES = 5;

export class DocumentStore {
  private docs = new Map<string, StoredDocument>();

  save(doc: StoredDocument): void {
    this.docs.set(doc.id, { ...doc, fullText: doc.fullText || doc.text });
  }

  get(docId: string): StoredDocument | undefined {
    return this.docs.get(docId);
  }

  ids(): string[] {
    return [...this.docs.keys()];
  }

  /** Read a bounded passage window by page, section, or passage range. */
  open(docId: string, options: { page?: number; section?: string; passage?: number; offset?: number } = {}): Passage[] {
    const doc = this.docs.get(docId);
    if (!doc) throw new Error('Unknown document in this session');
    const fullText = doc.fullText || doc.text;
    if (!fullText.trim()) throw new Error(`Document ${docId} has no stored text`);

    if (typeof options.page === 'number' && doc.pages?.length) {
      const index = Math.max(1, Math.min(doc.pages.length, Math.floor(options.page))) - 1;
      return [{
        ref: `${docId}:page:${index + 1}`,
        docId,
        page: index + 1,
        text: doc.pages[index].slice(0, MAX_PASSAGE_CHARS),
        chars: Math.min(doc.pages[index].length, MAX_PASSAGE_CHARS),
      }];
    }

    if (options.section) {
      const needle = options.section.toLowerCase();
      const chunks = chunkDocument({ ...doc, text: fullText }, { targetTokens: 500, overlapTokens: 60 });
      const hit = chunks.find((c) => (c.headingPath || []).join(' ').toLowerCase().includes(needle));
      if (hit) {
        return [{
          ref: `${docId}:section:${(hit.headingPath || ['untitled']).join('>')}`,
          docId,
          section: (hit.headingPath || []).join(' > '),
          text: hit.text.slice(0, MAX_PASSAGE_CHARS),
          chars: Math.min(hit.text.length, MAX_PASSAGE_CHARS),
        }];
      }
    }

    const chunks = chunkDocument({ ...doc, text: fullText }, { targetTokens: 500, overlapTokens: 60 });
    const index = Math.max(0, (options.passage ?? options.offset ?? 1) - 1);
    return chunks.slice(index, index + 1).map((chunk, i) => ({
      ref: `${docId}:passage:${index + i + 1}`,
      docId,
      section: (chunk.headingPath || []).join(' > ') || undefined,
      text: chunk.text.slice(0, MAX_PASSAGE_CHARS),
      chars: Math.min(chunk.text.length, MAX_PASSAGE_CHARS),
    }));
  }

  /** Document-local search over full stored text with surrounding context. */
  find(docId: string, term: string, contextChars = 400): Passage[] {
    const doc = this.docs.get(docId);
    if (!doc) throw new Error('Unknown document in this session');
    const fullText = doc.fullText || doc.text;
    const needle = term.toLowerCase();
    if (!needle.trim()) throw new Error('Search term must not be empty');
    const out: Passage[] = [];
    let from = 0;
    let hitIndex = 0;
    while (out.length < MAX_PASSAGES) {
      const at = fullText.toLowerCase().indexOf(needle, from);
      if (at < 0) break;
      hitIndex++;
      const start = Math.max(0, at - contextChars);
      const end = Math.min(fullText.length, at + needle.length + contextChars);
      out.push({
        ref: `${docId}:match:${hitIndex}`,
        docId,
        text: fullText.slice(start, end),
        chars: end - start,
      });
      from = at + needle.length;
    }
    return out;
  }

  passageStats(): { documents: number; chars: number; tokens: number } {
    let chars = 0;
    for (const doc of this.docs.values()) chars += (doc.fullText || doc.text).length;
    return { documents: this.docs.size, chars, tokens: defaultTokenCounter.count([...this.docs.values()].map((d) => d.fullText || d.text).join('\n')) };
  }
}

/** Minimal PDF text extraction: uncompresses stream objects and pulls text
 *  spans with approximate page attribution. No OCR dependency is bundled, so
 *  scanned PDFs (no extractable text) fail explicitly with `ocr-required`. */
export function extractPdfText(bytes: Uint8Array, title = ''): {
  pages: string[];
  text: string;
  needsOcr: boolean;
} {
  const raw = new TextDecoder('latin1').decode(bytes);
  const pages: string[] = [];
  // Split raw content at page markers first so FlateDecode filtering applies
  // per page; then pull text spans from each page body.
  const pageBodies = raw.split(/\/Type\s*\/Page[^s]/g).slice(1);
  const bodies = pageBodies.length > 0 ? pageBodies : [raw];
  const pageTexts: string[] = bodies.map((body) => {
    const spans: string[] = [];
    const streamRe = /stream\r?\n([\s\S]*?)\r?\nendstream/g;
    let match: RegExpExecArray | null;
    while ((match = streamRe.exec(body)) !== null) {
      const preceding = body.slice(Math.max(0, match.index - 500), match.index);
      void preceding;
      const texts = [...match[1].matchAll(/\((?:\\.|[^()\\])*\)\s*Tj|<(?:[0-9a-fA-F\s]+)>\s*Tj/g)].map((m) => decodePdfSpan(m[0]));
      if (texts.length) spans.push(texts.join(' '));
    }
    // Bodies without stream wrappers (test fixtures, linearized PDFs).
    if (spans.length === 0) {
      const texts = [...body.matchAll(/\((?:\\.|[^()\\])*\)\s*Tj|<(?:[0-9a-fA-F\s]+)>\s*Tj/g)].map((m) => decodePdfSpan(m[0]));
      if (texts.length) spans.push(texts.join(' '));
    }
    return spans.join('\n').trim();
  });
  for (const text of pageTexts) pages.push(text);
  const text = pageTexts.join('\n').replace(/[ \t]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
  void title;
  return { pages, text, needsOcr: text.length < 50 };
}

function decodePdfSpan(span: string): string {
  if (span.startsWith('(')) {
    return span
      .slice(1, span.lastIndexOf(')'))
      .replace(/\\n/g, '\n')
      .replace(/\\r/g, '\r')
      .replace(/\\t/g, '\t')
      .replace(/\\\(/g, '(')
      .replace(/\\\)/g, ')')
      .replace(/\\\\/g, '\\');
  }
  const hex = span.slice(1, span.indexOf('>')).replace(/\s+/g, '');
  let out = '';
  for (let i = 0; i + 3 < hex.length; i += 4) {
    const code = parseInt(hex.slice(i, i + 4), 16);
    if (Number.isFinite(code) && code > 0) out += String.fromCharCode(code);
  }
  return out;
}
