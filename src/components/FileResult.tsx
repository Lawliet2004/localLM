import { useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { CodeBlock } from './CodeBlock';
import { languageFromPath } from '../lib/highlight';

export function FileResult({ path, content, totalLines, lineRange, sha256, raw }: { path: string; content: string; totalLines?: number; lineRange?: string; sha256?: string; raw: unknown }) {
  const [open, setOpen] = useState(false);
  const [inspected, setInspected] = useState(false);
  const [rawOpen, setRawOpen] = useState(false);
  const lineCount = !lineRange && typeof totalLines === 'number' && totalLines >= 0 ? totalLines : undefined;
  const returnedLines = content ? content.replace(/\r?\n$/, '').split('\n').length : 0;
  return <>
  <details className="file-result" open={open} onToggle={event => { setOpen(event.currentTarget.open); if (event.currentTarget.open) setInspected(true); }}>
    <summary aria-label={`Read file ${path}${lineRange ? ` (${lineRange})` : ''}`} onClick={event => { event.preventDefault(); setInspected(true); setOpen(value => !value); }}>
      <span className="tool-dot" aria-hidden="true" />
      <span className="file-result-label"><strong>Read</strong><span className="tool-target">(<code className="file-result-path">{path}</code>)</span></span>
      <ChevronRight size={14} className="trace-chevron" aria-hidden="true" />
    </summary>
    {(open || inspected) && <div className="file-result-content">
      <CodeBlock code={content} language={languageFromPath(path)} defaultWrap className="file-read-code" ariaLabel={`Contents of ${path}`} />
      <details open={rawOpen} onToggle={event => setRawOpen(event.currentTarget.open)}>
        <summary onClick={event => { event.preventDefault(); setRawOpen(value => !value); }}>Request and raw result</summary>
        {rawOpen && <>
          {typeof totalLines === 'number' && <p className="file-fingerprint">{totalLines} total lines</p>}
          {sha256 && <p className="file-fingerprint">SHA-256 <code>{sha256}</code></p>}
          <CodeBlock code={JSON.stringify(raw, null, 2)} language="json" defaultWrap className="file-read-code" ariaLabel={`Raw result for ${path}`} />
        </>}
      </details>
    </div>}
  </details>
  {!open && <p className="activity-excerpt transcript-branch">Read {returnedLines} {returnedLines === 1 ? 'line' : 'lines'}{lineCount !== undefined && lineCount > returnedLines ? ` of ${lineCount}` : ''}</p>}
  </>;
}
