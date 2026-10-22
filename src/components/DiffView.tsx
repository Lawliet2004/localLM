import { Fragment, memo, useMemo, useState } from 'react';
import { highlightCode, languageFromPath } from '../lib/highlight';

type DiffRow = { text: string; kind: 'add' | 'del' | 'hdr' | ''; oldLine: number | ''; newLine: number | '' };
function diffRows(diff: string): DiffRow[] {
  let oldLine = 0, newLine = 0, oldRemaining = 0, newRemaining = 0;
  return diff.split('\n').map(text => {
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(text);
    if (hunk) {
      oldLine = Number(hunk[1]); oldRemaining = Number(hunk[2] ?? 1);
      newLine = Number(hunk[3]); newRemaining = Number(hunk[4] ?? 1);
      return { text, kind: 'hdr', oldLine: '', newLine: '' };
    }
    if (text.startsWith('diff ')) oldRemaining = newRemaining = 0;
    const inHunk = oldRemaining > 0 || newRemaining > 0;
    const add = inHunk && text.startsWith('+'), del = inHunk && text.startsWith('-');
    const context = inHunk && text.startsWith(' ');
    const oldNumber = del || context ? oldLine++ : '';
    const newNumber = add || context ? newLine++ : '';
    if (del || context) oldRemaining--;
    if (add || context) newRemaining--;
    return { text, kind: add ? 'add' : del ? 'del' : '', oldLine: oldNumber, newLine: newNumber };
  });
}

/** One diff surface, with real source coordinates and safe syntax highlighting.
 * The source +/- markers remain visible so meaning never depends on color. */
export const DiffLines = memo(function DiffLines({ diff, path }: { diff: string; path?: string }) {
  const rows = useMemo(() => diffRows(diff), [diff]);
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
  const regions = useMemo(() => {
    const regions = new Map<number, number>();
    for (let index = 0; index < rows.length;) {
      const start = index;
      while (index < rows.length && rows[index].oldLine !== '' && rows[index].newLine !== '') index++;
      if (index - start > 8) regions.set(start + 3, index - 3);
      if (index === start) index++;
    }
    return regions;
  }, [rows]);
  const folded = new Set<number>();
  for (const [start, end] of regions) if (!expanded.has(start)) for (let index = start; index < end; index++) folded.add(index);
  const language = languageFromPath(path ?? /^\+\+\+ (?:b\/)?(.+)$/m.exec(diff)?.[1] ?? '');
  const highlight = diff.length <= 100_000;
  return <pre className="tool-diff diff-lines" tabIndex={0} aria-label="File changes">
    {rows.map((row, idx) => {
      const regionEnd = regions.get(idx);
      const isExpanded = expanded.has(idx);
      if (folded.has(idx) && regionEnd === undefined) return null;
      return <Fragment key={idx}>
        {regionEnd !== undefined && <div className="diff-context-control"><button type="button" aria-expanded={isExpanded} onClick={() => setExpanded(current => {
          const next = new Set(current);
          if (next.has(idx)) next.delete(idx); else next.add(idx);
          return next;
        })}>{isExpanded ? 'Collapse unchanged lines' : `Show ${regionEnd - idx} unchanged lines`}</button></div>}
        {!folded.has(idx) && <div className={`diff-line${row.kind ? ` ${row.kind}` : ''}`}>
      <span className="diff-gutter" aria-hidden="true">{row.oldLine}</span><span className="diff-gutter" aria-hidden="true">{row.newLine}</span>
      {highlight && language && (row.kind === 'add' || row.kind === 'del') ? <span className="diff-content"><span>{row.text[0]}</span><span dangerouslySetInnerHTML={{ __html: highlightCode(row.text.slice(1), language).html }} /></span> : <span className="diff-content">{row.text}</span>}
    </div>}</Fragment>; })}
  </pre>;
});

/** Counts recorded hunk lines, including source beginning with ++ or --. */
export function diffCounts(diff: string): { added: number; removed: number } {
  return diffRows(diff).reduce((counts, row) => ({ added: counts.added + Number(row.kind === 'add'), removed: counts.removed + Number(row.kind === 'del') }), { added: 0, removed: 0 });
}
