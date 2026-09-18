import { describe, it, expect } from 'vitest';
import { validateAndCleanCitations, renderMarkdownCitations } from './citation_renderer';
import type { SourceRegistry } from '../types';

describe('Citation Renderer & Validator', () => {
  const sources: SourceRegistry = {
    S1: {
      id: 'S1',
      title: 'React Official Blog',
      url: 'https://react.dev/blog/2024/12/05/react-19',
      domain: 'react.dev',
    },
    S2: {
      id: 'S2',
      title: 'Next.js 15 Release Notes',
      url: 'https://nextjs.org/blog/next-15',
      domain: 'nextjs.org',
    },
  };

  it('validates citations and strips hallucinated IDs (e.g. S99)', () => {
    const text = 'React 19 was released [S1]. An unverified claim [S99]. Another fact [S1, S2].';
    const result = validateAndCleanCitations(text, sources);

    expect(result.valid).toBe(false); // Because S99 was present
    expect(result.invalidSourceIds).toEqual(['S99']);
    expect(result.citedSourceIds).toEqual(['S1', 'S2']);
    expect(result.cleanedText).not.toContain('S99');
    expect(result.cleanedText).toContain('[S1]');
    expect(result.cleanedText).toContain('[S1, S2]');
  });

  it('transforms [S1] into markdown hyperlinks using application source registry', () => {
    const text = 'React 19 is out [S1] and Next.js 15 is available [S2].';
    const rendered = renderMarkdownCitations(text, sources);

    expect(rendered).toContain('[[S1]](https://react.dev/blog/2024/12/05/react-19)');
    expect(rendered).toContain('[[S2]](https://nextjs.org/blog/next-15)');
  });

  it('transforms multi-source citations [S1, S2] into individual markdown hyperlinks', () => {
    const text = 'Both frameworks were updated recently [S1, S2].';
    const rendered = renderMarkdownCitations(text, sources);

    expect(rendered).toContain('[[S1]](https://react.dev/blog/2024/12/05/react-19)');
    expect(rendered).toContain('[[S2]](https://nextjs.org/blog/next-15)');
    expect(rendered).not.toContain('[S1, S2]');
  });
});
