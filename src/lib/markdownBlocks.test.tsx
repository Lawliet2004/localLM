import { Fragment } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { expect, it } from 'vitest';
import { MarkdownBlockBuffer } from './markdownBlocks';

const fixtures = [
  '# Heading\n\nFirst **paragraph**.\n\nSecond paragraph.\n\n- One\n- Two\n\nEnd.',
  'Setext\n===\n\nNext\n---\n\nEnd\n\nTail',
  '- One\n\n  Continued paragraph\n\n- Two\n  - Nested\n\nAfter\n\nTail',
  '> quote\nlazy continuation\n\n> another\n\nAfter\n\nTail',
  'First\n\n```ts\nconst n = 1;\n\nconst m = 2;\n```\n\nAfter\n\nTail',
  '| A | B |\n|---|---|\n| 1 | 2 |\n\nAfter\n\nTail',
  'Text [ref].\n\nAfter\n\n[ref]: https://example.com\n',
  'Text[^1].\n\nAfter\n\n[^1]: Footnote\n',
  'Text [foo bar].\n\nAfter\n\n[foo\nbar]: https://example.com\n',
  'Text [foo\\]].\n\nAfter\n\n[foo\\]]: https://example.com\n',
  '<div>\n\nText\n\n</div>\n\nAfter',
  '- [x] Done\n- [ ] Pending\n\nAfter\n\nTail',
];
it.each(fixtures)('preserves canonical GFM rendering for streamed prefixes: %s', source => {
  const buffer = new MarkdownBlockBuffer();
  for (let size = 1; size <= source.length; size++) {
    const text = source.slice(0, size);
    const blocks = buffer.update(text);
    expect(blocks.map(block => block.content).join('')).toBe(text);
    const expected = renderToStaticMarkup(<ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>);
    const actual = renderToStaticMarkup(<>{blocks.map((block, index) => <Fragment key={block.offset}>{index > 0 ? '\n' : null}<ReactMarkdown remarkPlugins={[remarkGfm]}>{block.content}</ReactMarkdown></Fragment>)}</>);
    expect(actual, `prefix ${size}`).toBe(expected);
  }
});
it('reuses completed boundaries and handles replacement and global constructs safely', () => {
  const buffer = new MarkdownBlockBuffer();
  const source = Array.from({ length: 1000 }, (_, i) => `Paragraph ${i}.\n\n`).join('');
  let blocks = buffer.update(source);
  const first = blocks[0], fullParseWork = source.length * 100;
  for (let i = 0; i < 100; i++) blocks = buffer.update(`${source}${'New words '.repeat(i + 1)}`);
  expect(blocks[0]).toBe(first);
  expect(buffer.parsedCharacters).toBeLessThan(fullParseWork / 10);
  expect(buffer.update('Replacement').map(block => block.content)).toEqual(['Replacement']);
  expect(buffer.update('Replacement\n\n$$x$$')).toHaveLength(1);
});
