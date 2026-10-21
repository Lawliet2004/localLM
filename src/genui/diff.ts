export interface DiffRow { before?: string; after?: string; oldLine?: number; newLine?: number; heading?: string }
/** Align each contiguous deletion/addition group within a parsed unified hunk.
 * Hunk/file boundaries are explicit rows; gaps never acquire fictitious lines. */
export function alignedDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = []; let oldLine = 0, newLine = 0, oldRemaining = 0, newRemaining = 0, inHunk = false;
  let removed: string[] = [], added: string[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) rows.push({ before: removed[i], after: added[i], oldLine: i < removed.length ? oldLine++ : undefined, newLine: i < added.length ? newLine++ : undefined });
    removed = []; added = [];
  };
  for (const line of diff.split('\n')) {
    if (inHunk && oldRemaining === 0 && newRemaining === 0) { flush(); inHunk = false; }
    const hunk = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (hunk) { flush(); oldLine = Number(hunk[1]); newLine = Number(hunk[3]); oldRemaining = Number(hunk[2] ?? 1); newRemaining = Number(hunk[4] ?? 1); inHunk = [oldLine,newLine,oldRemaining,newRemaining].every(Number.isSafeInteger) && oldRemaining<=10000 && newRemaining<=10000; rows.push({heading:line}); }
    else if (!inHunk && /^(?:diff |--- |\+\+\+ )/.test(line)) { flush(); inHunk = false; rows.push({heading:line}); }
    else if (inHunk && oldRemaining>0 && line.startsWith('-')) { removed.push(line.slice(1)); oldRemaining--; }
    else if (inHunk && newRemaining>0 && line.startsWith('+')) { added.push(line.slice(1)); newRemaining--; }
    else { flush(); if (inHunk && oldRemaining>0 && newRemaining>0 && line.startsWith(' ')) { rows.push({before:line.slice(1),after:line.slice(1),oldLine:oldLine++,newLine:newLine++}); oldRemaining--; newRemaining--; } else if (line) { rows.push({heading:line}); if (!line.startsWith('\\')) inHunk = false; } }
  }
  flush(); return rows;
}
