import { describe, expect, it } from 'vitest';
import { mathMarkdown } from './mathMarkdown';

describe('model math delimiters', () => {
  it('normalizes complete inline and display LaTeX', () => {
    expect(mathMarkdown('Use \\(x^2\\).\n\\[\\frac{a}{b}\\]')).toBe('Use $$x^2$$.\n\n$$\n\\frac{a}{b}\n$$\n');
  });
  it.each([
    '```python\nexpression = "\\(x\\)"\n```',
    '```python\nexpression = "\\(x\\)"',
    'Use `\\(x\\)` as delimiters.',
    '    \\[literal\\]',
    '[docs](https://example.com/\\(x\\))',
    '<span title="\\(x\\)">text</span>',
    '\\\\(escaped\\)',
    'Partial \\(x',
    'Plain $5 and $10.',
  ])('preserves literal content: %s', source => {
    expect(mathMarkdown(source)).toBe(source);
  });
});
