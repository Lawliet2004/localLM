import { expect, it } from 'vitest';
import { extractStructuredContent } from './structured_content';
it('reads JSON literally without removing markup inside string values', () => {
  const result = extractStructuredContent('{"content":"<script>evidence</script>","count":42}', 'application/json');
  expect(JSON.parse(result.text)).toEqual({ content: '<script>evidence</script>', count: 42 });
  expect(() => extractStructuredContent('{broken', 'application/json')).toThrow();
});
it('reads CSV quoted delimiters, Unicode, multiline fields and escaped quotes', () => {
  const result = extractStructuredContent('\uFEFFname,note\r\n"世界","one, two\nthree ""quoted"""\r\n', 'text/csv');
  expect(result.rows).toBe(2);
  expect(result.text).toContain('Row 2: ["世界","one, two\\nthree \\"quoted\\""]');
  expect(extractStructuredContent('a\tb\n1\t2', 'text/tab-separated-values').rows).toBe(2);
  expect(() => extractStructuredContent('"unclosed', 'text/csv')).toThrow('unterminated');
  expect(() => extractStructuredContent('"closed"bad', 'text/csv')).toThrow('unexpected');
});
