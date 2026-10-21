import { fromMarkdown } from 'mdast-util-from-markdown';

/** Models commonly emit LaTeX's \(inline\) and \[display\] delimiters.
 * Normalize only complete expressions outside literal source and URLs.
 * The Markdown parser supplies the literal ranges, including unclosed fences,
 * so a regular expression can never rewrite code copied from an answer. */
export function mathMarkdown(source: string): string {
  if (!source.includes('\\(') && !source.includes('\\[')) return source;
  const protectedRanges: [number, number][] = [];
  const tree = fromMarkdown(source);
  function visit(node: typeof tree | (typeof tree.children)[number]): void {
    if (['code', 'inlineCode', 'html', 'link', 'image', 'definition'].includes(node.type)) {
      const start = node.position?.start.offset;
      const end = node.position?.end.offset;
      if (start !== undefined && end !== undefined) protectedRanges.push([start, end]);
      return;
    }
    if ('children' in node) node.children.forEach(visit);
  }
  visit(tree);
  return source.replace(/(?<!\\)\\\(([\s\S]*?)\\\)|(?<!\\)\\\[([\s\S]*?)\\\]/g, (match: string, inline: string | undefined, display: string | undefined, offset: number) => {
    if (protectedRanges.some(([start, end]) => offset < end && offset + match.length > start)) return match;
    return display !== undefined ? `\n$$\n${display.trim()}\n$$\n` : `$$${inline}$$`;
  });
}
