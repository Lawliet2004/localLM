import { describe, it, expect } from 'vitest';
import { extractMainContent } from './main_content';
import { DocumentStore, extractPdfText } from '../documents/document_store';

describe('full-document reading', () => {
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
    expect(pdf.pages.length).toBe(2);
    expect(pdf.pages[0]).toContain('Hello page one');
    expect(pdf.pages[1]).toContain('Hello page two');
    const scanned = extractPdfText(new TextEncoder().encode('1 0 obj /Type /Page >> stream\nendstream endobj'));
    expect(scanned.needsOcr).toBe(true);
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
