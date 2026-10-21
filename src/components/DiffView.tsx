/** Shared unified-diff lines used by tool cards and the work summary. */
export function DiffLines({ diff }: { diff: string }) {
  return (
    <pre className="tool-diff diff-lines">
      {diff.split('\n').map((line, idx) => {
        const isAdd = line.startsWith('+') && !line.startsWith('+++');
        const isDel = line.startsWith('-') && !line.startsWith('---');
        const isHdr = line.startsWith('@@');
        return (
          <div key={idx} className={isAdd ? 'diff-line add' : isDel ? 'diff-line del' : isHdr ? 'diff-line hdr' : 'diff-line'}>
            <span className="diff-gutter">{idx + 1}</span>
            <span className="diff-content">{line}</span>
          </div>
        );
      })}
    </pre>
  );
}

/** Count +/- lines inside hunks for summary badges. */
export function diffCounts(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  let inHunk = false;
  for (const line of diff.split('\n')) {
    if (line.startsWith('@@')) inHunk = true;
    else if (inHunk && line.startsWith('+') && !line.startsWith('+++')) added++;
    else if (inHunk && line.startsWith('-') && !line.startsWith('---')) removed++;
  }
  return { added, removed };
}
