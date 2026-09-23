/**
 * Document Store: full stored document text with stable evidence references.
 *
 * The worker keeps full documents outside model context (SQLite `research`
 * rows) and serves bounded passages with stable `docId:passage` references.
 * Open/find tools read from this store — never only from previously
 * compressed evidence — so facts excluded from the initial summary stay
 * recoverable. PDF text carries page references. Missing text is
 * `no_extractable_text`; it is not labeled OCR-required.
 */

import { inflateSync, unzlibSync } from 'fflate';
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

export type PdfStatus =
  | 'text_extracted'
  | 'no_extractable_text'
  | 'password_required'
  | 'invalid_document'
  | 'limit_exceeded';

const PDF_BYTE_LIMIT = 8_000_000;
const PDF_INFLATE_LIMIT = 2_000_000;

class PdfLimitExceeded extends Error {
  constructor() {
    super('PDF inflate limit exceeded');
    this.name = 'PdfLimitExceeded';
  }
}

/** Local PDF text extraction.
 *  Bytes are parsed in-process. `title` is ignored so a remote URL is never
 *  fetched or parsed, and external PDF resources are not loaded.
 *  `needsOcr` stays false: little or missing text is not labeled OCR-required. */
export function extractPdfText(bytes: Uint8Array, title = ''): {
  pages: string[];
  text: string;
  needsOcr: boolean;
  status: PdfStatus;
} {
  void title;
  if (bytes.length > PDF_BYTE_LIMIT) return pdfResult('limit_exceeded');
  // TextDecoder('latin1') is windows-1252 in browsers and jsdom, which remaps
  // 0x80-0x9F and breaks FlateDecode bytes such as the 0x9C zlib flag.
  const raw = bytesToLatin1(bytes);
  if (/\/Encrypt\s*(?:\d+\s+\d+\s+R|<<)/.test(raw)) return pdfResult('password_required');
  if (!looksLikePdf(raw)) return pdfResult('invalid_document');
  try {
    const budget = { used: 0 };
    const pages = pageBodies(raw).map((body) => normalizePdfText(pageText(body, budget)));
    const text = normalizePdfText(pages.join('\n'));
    return {
      pages,
      text,
      needsOcr: false,
      status: text.length > 0 ? 'text_extracted' : 'no_extractable_text',
    };
  } catch (error) {
    if (error instanceof PdfLimitExceeded) return pdfResult('limit_exceeded');
    throw error;
  }
}

function pdfResult(status: Exclude<PdfStatus, 'text_extracted'>): {
  pages: string[];
  text: string;
  needsOcr: boolean;
  status: PdfStatus;
} {
  return { pages: [], text: '', needsOcr: false, status };
}

function looksLikePdf(raw: string): boolean {
  const hasPage = /\/Type\s*\/Page(?!s)/.test(raw);
  const hasTextOp = /(?:^|[^A-Za-z])(?:Tj|TJ)(?![A-Za-z])/.test(raw);
  if (hasPage || hasTextOp) return true;
  if (!raw.includes('%PDF')) return false;
  return /\b\d+\s+\d+\s+obj\b/.test(raw) || /(?:^|[^A-Za-z])stream(?![A-Za-z])/.test(raw);
}

function normalizePdfText(text: string): string {
  return text.replace(/[ \t]+/g, ' ').replace(/\n{3,}/g, '\n\n').trim();
}

function pageBodies(raw: string): string[] {
  const ranges = streamRanges(raw);
  const markers: number[] = [];
  const re = /\/Type\s*\/Page(?!s)/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(raw)) !== null) {
    if (!indexInRanges(ranges, match.index)) markers.push(match.index);
  }
  if (markers.length === 0) return [raw];
  return markers.map((marker, index) => {
    const objAt = raw.lastIndexOf('obj', marker);
    const start = objAt >= 0
      && marker - objAt < 4000
      && !indexInRanges(ranges, objAt)
      && (index === 0 || objAt > markers[index - 1])
      ? objAt
      : marker;
    return raw.slice(start, markers[index + 1] ?? raw.length);
  });
}

function pageText(body: string, budget: { used: number }): string {
  const spans: string[] = [];
  let sawStream = false;
  let from = 0;
  while (from < body.length) {
    const at = findKeyword(body, 'stream', from);
    if (at < 0) break;
    const dataAt = streamDataOffset(body, at);
    if (dataAt < 0) {
      from = at + 1;
      continue;
    }
    const end = findKeyword(body, 'endstream', dataAt);
    if (end < 0) break;
    sawStream = true;
    const rawPayload = body.slice(dataAt, end);
    const trimmed = trimOneEol(rawPayload);
    const dict = dictionaryBefore(body, at);
    let decoded = trimmed;
    if (/\/FlateDecode\b/.test(dict) && !/\/Subtype\s*\/Image\b/.test(dict)) {
      const inflated = inflatePdfStream(trimmed) ?? (trimmed === rawPayload ? null : inflatePdfStream(rawPayload));
      if (inflated === 'limit') throw new PdfLimitExceeded();
      if (typeof inflated !== 'string') {
        from = end + 'endstream'.length;
        continue;
      }
      budget.used += inflated.length;
      if (budget.used > PDF_INFLATE_LIMIT) throw new PdfLimitExceeded();
      decoded = inflated;
    }
    const texts = collectSpans(decoded);
    if (texts.length) spans.push(texts.join(lineGap(decoded)));
    from = end + 'endstream'.length;
  }
  if (!sawStream) {
    const texts = collectSpans(body);
    if (texts.length) spans.push(texts.join(lineGap(body)));
  }
  return spans.join('\n').trim();
}

function dictionaryBefore(body: string, streamAt: number): string {
  const window = body.slice(Math.max(0, streamAt - 16000), streamAt);
  let cut = 0;
  for (const marker of ['endstream', 'endobj', 'obj']) {
    const at = window.lastIndexOf(marker);
    if (at > cut) cut = at;
  }
  return window.slice(cut);
}

function streamRanges(raw: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  let from = 0;
  while (from < raw.length) {
    const at = findKeyword(raw, 'stream', from);
    if (at < 0) break;
    const dataAt = streamDataOffset(raw, at);
    if (dataAt < 0) {
      from = at + 1;
      continue;
    }
    const end = findKeyword(raw, 'endstream', dataAt);
    if (end < 0) break;
    ranges.push([dataAt, end]);
    from = end + 'endstream'.length;
  }
  return ranges;
}

function streamDataOffset(body: string, streamAt: number): number {
  const dataAt = streamAt + 'stream'.length;
  if (body.startsWith('\r\n', dataAt)) return dataAt + 2;
  if (body[dataAt] === '\n' || body[dataAt] === '\r') return dataAt + 1;
  return -1;
}

function trimOneEol(payload: string): string {
  if (payload.endsWith('\r\n')) return payload.slice(0, -2);
  if (payload.endsWith('\n') || payload.endsWith('\r')) return payload.slice(0, -1);
  return payload;
}

function findKeyword(body: string, word: string, from: number): number {
  let at = from;
  while (at < body.length) {
    const found = body.indexOf(word, at);
    if (found < 0) return -1;
    const prev = found === 0 ? ' ' : body[found - 1];
    const nextIndex = found + word.length;
    const next = body[nextIndex] ?? '';
    const followedByEndobj = word === 'endstream' && body.startsWith('endobj', nextIndex);
    if (isDelimiter(prev) && (next === '' || isDelimiter(next) || followedByEndobj)) return found;
    at = found + 1;
  }
  return -1;
}

function isDelimiter(ch: string): boolean {
  return ch === '' || /[^A-Za-z0-9]/.test(ch);
}

function indexInRanges(ranges: Array<[number, number]>, index: number): boolean {
  return ranges.some(([start, end]) => index >= start && index < end);
}

function lineGap(body: string): string {
  return /[-\d.]+\s+[-\d.]+\s+Td/.test(body) ? '\n' : ' ';
}

function collectSpans(body: string): string[] {
  const spans: string[] = [];
  let i = 0;
  while (i < body.length) {
    const ch = body[i];
    if (ch === '(') {
      const parsed = readLiteral(body, i);
      if (parsed.end <= i) {
        i++;
        continue;
      }
      i = parsed.end;
      if (showsText(body, i)) spans.push(parsed.text);
      continue;
    }
    if (ch === '<' && body[i + 1] !== '<') {
      const parsed = readHex(body, i);
      if (!parsed || parsed.end <= i) {
        i++;
        continue;
      }
      i = parsed.end;
      if (showsText(body, i)) spans.push(parsed.text);
      continue;
    }
    if (ch === '[') {
      const parsed = readTextArray(body, i);
      if (parsed && parsed.end > i) {
        if (parsed.text) spans.push(parsed.text);
        i = parsed.end;
        continue;
      }
    }
    i++;
  }
  return spans.filter((span) => span.length > 0);
}

function showsText(body: string, index: number): boolean {
  const match = /^[ \t\r\n]*([A-Za-z'"]+)/.exec(body.slice(index));
  return match != null && (match[1] === 'Tj' || match[1] === "'" || match[1] === '"');
}

function readTextArray(body: string, start: number): { text: string; end: number } | null {
  let i = start + 1;
  let text = '';
  while (i < body.length) {
    const ch = body[i];
    if (ch === '(') {
      const parsed = readLiteral(body, i);
      if (parsed.end <= i) return null;
      text += parsed.text;
      i = parsed.end;
      continue;
    }
    if (ch === '<' && body[i + 1] !== '<') {
      const parsed = readHex(body, i);
      if (parsed && parsed.end > i) {
        text += parsed.text;
        i = parsed.end;
        continue;
      }
    }
    if (ch === ']') {
      const match = /^[ \t\r\n]*(TJ)/.exec(body.slice(i + 1));
      if (!match) return null;
      return { text, end: i + 1 + match[0].length };
    }
    i++;
  }
  return null;
}

function readLiteral(body: string, start: number): { text: string; end: number } {
  let i = start + 1;
  let depth = 1;
  let raw = '';
  while (i < body.length && depth > 0) {
    const ch = body[i];
    if (ch === '\\') {
      raw += ch;
      if (i + 1 < body.length) raw += body[++i];
      i++;
      continue;
    }
    if (ch === '(') depth++;
    if (ch === ')') {
      depth--;
      if (depth === 0) break;
    }
    raw += ch;
    i++;
  }
  return { text: unescapeLiteral(raw), end: Math.min(body.length, i + 1) };
}

function readHex(body: string, start: number): { text: string; end: number } | null {
  if (body[start + 1] === '<') return null;
  let hex = '';
  let i = start + 1;
  while (i < body.length) {
    const ch = body[i];
    if (ch === '>') return { text: decodeHexString(hex), end: i + 1 };
    if (/[0-9a-fA-F]/.test(ch)) hex += ch;
    else if (!/\s/.test(ch)) return null;
    i++;
  }
  return null;
}

function unescapeLiteral(body: string): string {
  let out = '';
  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    if (ch !== '\\') {
      out += ch;
      continue;
    }
    const next = body[++i];
    if (next == null) break;
    if (next === 'n') out += '\n';
    else if (next === 'r') out += '\r';
    else if (next === 't') out += '\t';
    else if (next === 'b') out += '\b';
    else if (next === 'f') out += '\f';
    else if (next === '(' || next === ')' || next === '\\') out += next;
    else if (next === '\n') continue;
    else if (next === '\r') {
      if (body[i + 1] === '\n') i++;
    } else if (next >= '0' && next <= '7') {
      let oct = next;
      for (let k = 0; k < 2; k++) {
        const digit = body[i + 1];
        if (digit >= '0' && digit <= '7') {
          oct += digit;
          i++;
        } else break;
      }
      out += String.fromCharCode(Number.parseInt(oct, 8) & 0xff);
    } else out += next;
  }
  return decodeUtf16Bytes(out);
}

function decodeUtf16Bytes(text: string): string {
  if (text.length < 2 || text.charCodeAt(0) !== 0xfe || text.charCodeAt(1) !== 0xff) return text;
  let out = '';
  for (let i = 2; i + 1 < text.length; i += 2) {
    const code = (text.charCodeAt(i) << 8) | (text.charCodeAt(i + 1) & 0xff);
    if (code > 0) out += String.fromCharCode(code);
  }
  return out;
}

function decodeHexString(hex: string): string {
  if (hex.length % 2 === 1) hex += '0';
  if (!hex) return '';
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    const value = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (!Number.isFinite(value)) return '';
    bytes[i] = value;
  }
  if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) return utf16be(bytes.subarray(2));
  if (bytes.length >= 2 && bytes.length % 2 === 0 && mostlyUtf16Be(bytes)) return utf16be(bytes);
  return bytesToLatin1(bytes);
}

function utf16be(bytes: Uint8Array): string {
  let out = '';
  for (let i = 0; i + 1 < bytes.length; i += 2) {
    const code = (bytes[i] << 8) | bytes[i + 1];
    if (code > 0) out += String.fromCharCode(code);
  }
  return out;
}

function mostlyUtf16Be(bytes: Uint8Array): boolean {
  let zeros = 0;
  const pairs = bytes.length / 2;
  for (let i = 0; i < bytes.length; i += 2) if (bytes[i] === 0) zeros++;
  return zeros * 2 >= pairs;
}

function inflatePdfStream(latin1: string): string | 'limit' | null {
  if (!latin1) return null;
  const bytes = latin1ToBytes(latin1);
  const zlibbed = tryInflate(bytes, 'zlib');
  if (zlibbed === 'limit') return 'limit';
  if (zlibbed) return bytesToLatin1(zlibbed);
  const raw = tryInflate(bytes, 'raw');
  if (raw === 'limit') return 'limit';
  if (raw) return bytesToLatin1(raw);
  return null;
}

function tryInflate(bytes: Uint8Array, mode: 'zlib' | 'raw'): Uint8Array | 'limit' | null {
  try {
    const out = new Uint8Array(PDF_INFLATE_LIMIT + 1);
    const inflated = mode === 'zlib' ? unzlibSync(bytes, { out }) : inflateSync(bytes, { out });
    if (inflated.length > PDF_INFLATE_LIMIT) return 'limit';
    return inflated;
  } catch {
    return null;
  }
}

function bytesToLatin1(bytes: Uint8Array): string {
  let out = '';
  const chunk = 8192;
  for (let i = 0; i < bytes.length; i += chunk) {
    out += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk) as unknown as number[]);
  }
  return out;
}

function latin1ToBytes(latin1: string): Uint8Array {
  const bytes = new Uint8Array(latin1.length);
  for (let i = 0; i < latin1.length; i++) bytes[i] = latin1.charCodeAt(i) & 0xff;
  return bytes;
}
