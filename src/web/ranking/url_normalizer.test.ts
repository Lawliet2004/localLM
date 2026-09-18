import { describe, it, expect } from 'vitest';
import { normalizeUrl, extractDomain } from './url_normalizer';

describe('URL Normalizer', () => {
  it('removes tracking parameters (utm_*, fbclid, gclid)', () => {
    const raw = 'https://example.com/article?id=123&utm_source=twitter&utm_medium=social&fbclid=abc1234';
    const norm = normalizeUrl(raw);
    expect(norm).toBe('https://example.com/article?id=123');
  });

  it('strips www prefix, fragments, and default ports', () => {
    const raw = 'http://www.example.com:80/path/#heading';
    const norm = normalizeUrl(raw);
    expect(norm).toBe('http://example.com/path');
  });

  it('removes trailing slash on non-root paths and sorts parameters', () => {
    const raw = 'https://example.com/docs/?b=2&a=1';
    const norm = normalizeUrl(raw);
    expect(norm).toBe('https://example.com/docs?a=1&b=2');
  });

  it('extracts domain correctly', () => {
    expect(extractDomain('https://www.nature.com/articles/123')).toBe('nature.com');
    expect(extractDomain('https://docs.github.com/en')).toBe('docs.github.com');
  });
});
