import { useState } from 'react';
import { Copy, FileText, ChevronRight } from 'lucide-react';
import { CodeBlock } from './CodeBlock';
import { languageFromPath } from '../lib/highlight';

export function FileResult({ path, content, totalLines, sha256, raw }: { path: string; content: string; totalLines?: number; sha256?: string; raw: unknown }) {
  const [notice, setNotice] = useState('');
  const [open, setOpen] = useState(false);
  const [inspected, setInspected] = useState(false);
  const [rawOpen, setRawOpen] = useState(false);
  return <details className="file-result" open={open} onToggle={event => { setOpen(event.currentTarget.open); if (event.currentTarget.open) setInspected(true); }}>
    <summary onClick={event => { event.preventDefault(); setInspected(true); setOpen(value => !value); }}>
      <span className="trace-icon trace-icon-read" aria-hidden="true"><FileText size={14} /></span><span>Read <span className="file-result-path">{path}</span>{typeof totalLines === 'number' ? ` · ${totalLines} total lines` : ''}</span><ChevronRight size={14} className="trace-chevron" aria-hidden="true" />
    </summary>
    {(open || inspected) && <div className="file-result-content">
      <button type="button" className="secondary" aria-label={`Copy ${path} excerpt`} onClick={async () => { try { await navigator.clipboard.writeText(content); setNotice('Copied file excerpt.'); } catch { setNotice('Copy failed. Select the excerpt to copy it.'); } }}><Copy size={14} />Copy excerpt</button>
      <CodeBlock code={content} language={languageFromPath(path)} defaultWrap className="file-read-code" ariaLabel={`Contents of ${path}`} />
      {sha256 && <p className="file-fingerprint">SHA-256 <code>{sha256}</code></p>}
      <details open={rawOpen} onToggle={event => setRawOpen(event.currentTarget.open)}>
        <summary onClick={event => { event.preventDefault(); setRawOpen(value => !value); }}>Request and raw result</summary>
        {rawOpen && <CodeBlock code={JSON.stringify(raw, null, 2)} language="json" defaultWrap className="file-read-code" ariaLabel={`Raw result for ${path}`} />}
      </details>
      {notice && <p role="status">{notice}</p>}
    </div>}
  </details>;
}
