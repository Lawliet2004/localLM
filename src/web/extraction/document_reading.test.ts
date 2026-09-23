import { describe, it, expect } from 'vitest';
import { deflateSync, zlibSync } from 'fflate';
import { extractMainContent } from './main_content';
import { DocumentStore, extractPdfText } from '../documents/document_store';

function utf8(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

function concatBytes(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function pageWithStream(streamBody: Uint8Array, filter = ''): Uint8Array {
  const dict = filter ? `<< /Type /Page ${filter} /Length ${streamBody.length} >>` : '<< /Type /Page >>';
  return concatBytes([
    utf8(`1 0 obj ${dict} stream\n`),
    streamBody,
    utf8('\nendstream endobj\n'),
  ]);
}

describe('full-document reading', () => {
  it('prefers JSON-LD articleBody when HTML extraction is thin', () => {
    const body = 'The stable release is 19.0 and it shipped Actions plus useActionState. '.repeat(8);
    const html = `<html><head>
      <script type="application/ld+json">${JSON.stringify({
        '@type': 'TechArticle',
        headline: 'React 19',
        articleBody: body,
      })}</script>
    </head><body><nav>chrome</nav><p>short</p></body></html>`;
    const ext = extractMainContent(html, 'fallback');
    expect(ext.title).toBe('React 19');
    expect(ext.text).toContain('useActionState');
    expect(ext.text.length).toBeGreaterThan(200);
  });

  it('preserves headings, links, and table structure', () => {
    const html = `<html><head><title>Release</title></head><body>
      <h1>Model Release</h1><h2>Benchmarks</h2>
      <p>See <a href="https://example.com/card">the model card</a> for details.</p>
      <table><tr><th>Model</th><th>Score</th></tr><tr><td>Alpha</td><td>88.4</td></tr></table>
    </body></html>`;
    const ext = extractMainContent(html, 'fallback');
    expect(ext.headings).toContain('Model Release');
    expect(ext.headings).toContain('Benchmarks');
    expect(ext.links?.some((l) => l.href === 'https://example.com/card')).toBe(true);
    expect(ext.tables?.[0]).toBe('Model | Score');
    expect(ext.tables?.[1]).toBe('Alpha | 88.4');
    expect(ext.text).toContain('## Benchmarks');
  });

  it('open/find recover facts excluded from the initial evidence summary', () => {
    const store = new DocumentStore();
    const buried = 'The release notes mention version 9.9.9 only in paragraph twelve. ';
    store.save({
      id: 'doc-1', url: 'https://example.com/notes', domain: 'example.com',
      title: 'Notes', text: `Intro paragraph.\n\n${buried.repeat(20)}\n\nConclusion.`,
      fullText: `Intro paragraph.\n\n${buried.repeat(20)}\n\nConclusion.`,
      retrievedAt: new Date().toISOString(), searchResultIds: [],
      extractionMethod: 'main_content',
    });
    const found = store.find('doc-1', '9.9.9');
    expect(found.length).toBeGreaterThan(0);
    expect(found[0].text).toContain('9.9.9');
    expect(found[0].ref).toContain('doc-1:match:');
    const opened = store.open('doc-1', { passage: 1 });
    expect(opened[0].ref).toContain('doc-1:passage:');
  });

  it('reads by page and section with stable refs', () => {
    const store = new DocumentStore();
    store.save({
      id: 'doc-2', url: 'https://example.com/guide', domain: 'example.com',
      title: 'Guide', text: '# Install\n\nRun setup.\n\n# Usage\n\nRun the tool.',
      fullText: '# Install\n\nRun setup.\n\n# Usage\n\nRun the tool.',
      pages: ['Install page text', 'Usage page text'],
      retrievedAt: new Date().toISOString(), searchResultIds: [],
      extractionMethod: 'main_content',
    });
    const page = store.open('doc-2', { page: 2 });
    expect(page[0].page).toBe(2);
    expect(page[0].ref).toBe('doc-2:page:2');
    const section = store.open('doc-2', { section: 'usage' });
    expect(section[0].section).toContain('Usage');
  });

  it('keeps PDF page references and fails scanned PDFs explicitly', () => {
    const page1 = '(Hello page one and more extractable text for the page) Tj';
    const page2 = '(Hello page two and more extractable text for the page) Tj';
    const fake = `1 0 obj /Type /Page >> stream\n${page1}\nendstream endobj 2 0 obj /Type /Page >> stream\n${page2}\nendstream endobj`;
    const pdf = extractPdfText(new TextEncoder().encode(fake));
    expect(pdf.needsOcr).toBe(false);
    expect(pdf.status).toBe('text_extracted');
    expect(pdf.pages.length).toBe(2);
    expect(pdf.pages[0]).toContain('Hello page one');
    expect(pdf.pages[1]).toContain('Hello page two');
    expect(pdf.pages[0]).not.toContain('page two');
    expect(pdf.pages[1]).not.toContain('page one');
    const scanned = extractPdfText(new TextEncoder().encode('1 0 obj /Type /Page >> stream\nendstream endobj'));
    expect(scanned.status).toBe('no_extractable_text');
    expect(scanned.needsOcr).toBe(false);
    expect(scanned.text).toBe('');
  });

  it('inflates FlateDecode streams, including raw deflate', () => {
    const zlibPdf = extractPdfText(pageWithStream(zlibSync(utf8('(Compressed hello page) Tj')), '/Filter /FlateDecode'));
    expect(zlibPdf.status).toBe('text_extracted');
    expect(zlibPdf.needsOcr).toBe(false);
    expect(zlibPdf.text).toContain('Compressed hello page');
    const rawPdf = extractPdfText(pageWithStream(deflateSync(utf8('(Raw deflate hello) Tj')), '/Filter /FlateDecode'));
    expect(rawPdf.status).toBe('text_extracted');
    expect(rawPdf.text).toContain('Raw deflate hello');
  });

  it('reads TJ arrays and hex or Unicode spans without calling that OCR', () => {
    const tj = extractPdfText(utf8('1 0 obj << /Type /Page >> stream\n[(Hello) -20 ( TJ array)] TJ\n(line two) Tj\nendstream endobj'));
    expect(tj.status).toBe('text_extracted');
    expect(tj.needsOcr).toBe(false);
    expect(tj.text).toContain('Hello TJ array');
    expect(tj.text).toContain('line two');

    const hex = extractPdfText(utf8(
      '1 0 obj << /Type /Page >> stream\n<48 65 6C 6C 6F> Tj\n<FEFF00480069> Tj\n<0041> Tj\n(\\376\\377\\000B) Tj\nendstream endobj',
    ));
    expect(hex.status).toBe('text_extracted');
    expect(hex.needsOcr).toBe(false);
    expect(hex.text).toContain('Hello');
    expect(hex.text).toContain('Hi');
    expect(hex.text).toContain('A');
    expect(hex.text).toContain('B');

    const short = extractPdfText(utf8('1 0 obj << /Type /Page >> stream\n(Hi) Tj\nendstream endobj'));
    expect(short.status).toBe('text_extracted');
    expect(short.needsOcr).toBe(false);
    expect(short.text).toBe('Hi');
  });

  it('reports image-only, malformed, encrypted, and oversized PDFs without OCR', () => {
    const image = extractPdfText(utf8('1 0 obj << /Type /Page >> stream\nq 100 0 0 100 0 0 cm /Im0 Do Q\nendstream endobj'));
    expect(image.status).toBe('no_extractable_text');
    expect(image.needsOcr).toBe(false);
    expect(image.text).toBe('');

    expect(extractPdfText(utf8('this is not a pdf')).status).toBe('invalid_document');
    expect(extractPdfText(utf8('%PDF-1.4\nthis is truncated garbage')).status).toBe('invalid_document');
    expect(extractPdfText(new Uint8Array([1, 2, 3, 4, 255])).status).toBe('invalid_document');

    const encrypted = extractPdfText(utf8('%PDF-1.4\n1 0 obj << /Encrypt 2 0 R >> endobj\n2 0 obj << /Type /Page >> stream\n(Secret) Tj\nendstream endobj'));
    expect(encrypted.status).toBe('password_required');
    expect(encrypted.needsOcr).toBe(false);
    expect(encrypted.text).toBe('');
    expect(encrypted.pages).toEqual([]);

    const oversized = new Uint8Array(8_000_001);
    oversized.set(utf8('%PDF-1.4'), 0);
    const tooBig = extractPdfText(oversized);
    expect(tooBig.status).toBe('limit_exceeded');
    expect(tooBig.needsOcr).toBe(false);
    expect(tooBig.text).toBe('');

    const payload = new Uint8Array(2_000_001);
    payload.fill(0x41);
    const inflated = extractPdfText(pageWithStream(zlibSync(payload), '/Filter /FlateDecode'));
    expect(inflated.status).toBe('limit_exceeded');
    expect(inflated.needsOcr).toBe(false);
    expect(inflated.text).toBe('');
  });

  it('duplicate sources do not count as independent corroboration', async () => {
    const { deduplicateSearchResults } = await import('../ranking/deduplicator');
    const dupes = [
      { id: 'a', queryId: 'q', title: 'Breaking: Model X released today', url: 'https://a.example.com/x', domain: 'a.example.com', rank: 1 },
      { id: 'b', queryId: 'q', title: 'Breaking: Model X released today', url: 'https://b.example.com/x', domain: 'b.example.com', rank: 2 },
    ];
    expect(deduplicateSearchResults(dupes)).toHaveLength(1);
  });
});
