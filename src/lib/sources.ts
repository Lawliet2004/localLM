export function safeSourceUrl(value?: string): URL | null {
  if (!value) return null;
  try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) ? url : null; } catch { return null; }
}
export function sourceIdentity(url: URL): string { const copy = new URL(url); copy.hash = ''; return copy.href; }

type MarkdownNode = { type: string; value?: string; url?: string; children?: MarkdownNode[] };
/** Turn stored citation markers into links only when their ids resolve to real sources. */
export function citationPlugin(links: Record<string, string>) {
  return () => (tree: MarkdownNode) => {
    function visit(node: MarkdownNode) {
      if (!node.children || ['link', 'linkReference', 'code', 'inlineCode', 'html'].includes(node.type)) return;
      node.children = node.children.flatMap(child => {
        if (child.type !== 'text' || !child.value) { visit(child); return [child]; }
        const output: MarkdownNode[] = []; let cursor = 0;
        for (const match of child.value.matchAll(/\[([A-Za-z0-9_-]+)\]/g)) {
          const url = links[match[1]];
          if (!safeSourceUrl(url)) continue;
          const index = match.index!;
          if (index > cursor) output.push({ type: 'text', value: child.value.slice(cursor, index) });
          output.push({ type: 'link', url, children: [{ type: 'text', value: match[0] }] });
          cursor = index + match[0].length;
        }
        if (!cursor) return [child];
        if (cursor < child.value.length) output.push({ type: 'text', value: child.value.slice(cursor) });
        return output;
      });
    }
    visit(tree);
  };
}
