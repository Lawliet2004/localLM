import { fromMarkdown } from 'mdast-util-from-markdown';

export interface MarkdownBlock { offset: number; content: string; stable: boolean }

/** Boundaries only: rendering still uses the single react-markdown pipeline.
 * Keep two trailing CommonMark blocks mutable, and commit only at blank-line
 * boundaries. Document-wide constructs fall back to a single block so appended
 * definitions, footnotes, HTML and math can still change earlier content. */
export class MarkdownBlockBuffer {
  private source = '';
  private stable: MarkdownBlock[] = [];
  private offset = 0;
  parsedCharacters = 0;

  update(source: string): MarkdownBlock[] {
    if (!source.startsWith(this.source)) { this.stable = []; this.offset = 0; }
    this.source = source;
    // Labels may contain newlines and escaped closing brackets. A broad
    // string check is both conservative and linear, including across chunks.
    if (source.includes('[') && source.includes(']:') || /\[\^|\$\$|\\[([]|<[A-Za-z!/]/.test(source)) {
      this.stable = []; this.offset = 0;
      return [{ offset: 0, content: source, stable: false }];
    }
    const tail = source.slice(this.offset);
    this.parsedCharacters += tail.length;
    const nodes = fromMarkdown(tail).children;
    // Render each known blank-line block immediately, with stable offset keys.
    // Only the safe prefix enters the cache; the two trailing AST blocks are
    // rechecked for continuation/setext/list changes on the next append.
    const boundaries: { end: number; stable: boolean }[] = [];
    for (let index = 0; index < nodes.length - 1; index++) {
      const end = nodes[index].position?.end.offset;
      const next = nodes[index + 1].position?.start.offset;
      if (end !== undefined && next !== undefined && /\r?\n[\t ]*\r?\n/.test(tail.slice(end, next))) boundaries.push({ end: next, stable: index < nodes.length - 2 });
    }
    const result = [...this.stable];
    let consumed = 0, committed = 0;
    for (const boundary of boundaries) {
      const block = { offset: this.offset + consumed, content: tail.slice(consumed, boundary.end), stable: boundary.stable };
      result.push(block);
      if (boundary.stable) { this.stable.push(block); committed = boundary.end; }
      consumed = boundary.end;
    }
    result.push({ offset: this.offset + consumed, content: tail.slice(consumed), stable: false });
    this.offset += committed;
    return result;
  }
}
