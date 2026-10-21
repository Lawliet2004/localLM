import { useState } from 'react';
import { Copy, FileText } from 'lucide-react';
import { CodeBlock } from './CodeBlock';
import { languageFromPath } from '../lib/highlight';
export function FileResult({ path, content, totalLines, sha256, raw }: { path: string; content: string; totalLines?: number; sha256?: string; raw: unknown }) {
  const [notice, setNotice] = useState('');
  return <details className="file-result"><summary><FileText size={15} aria-hidden="true" />Read {path}{typeof totalLines === 'number' ? ` · ${totalLines} total lines` : ''}</summary><div className="file-result-content"><button className="secondary" aria-label={`Copy ${path} excerpt`} onClick={async () => { try { await navigator.clipboard.writeText(content); setNotice('Copied file excerpt.'); } catch { setNotice('Copy failed. Select the excerpt to copy it.'); } }}><Copy size={14} />Copy excerpt</button><CodeBlock code={content} language={languageFromPath(path)} defaultWrap className="file-read-code" ariaLabel={`Contents of ${path}`} />{sha256 && <p className="file-fingerprint">SHA-256 <code>{sha256}</code></p>}<details><summary>Request and raw result</summary><CodeBlock code={JSON.stringify(raw, null, 2)} language="json" defaultWrap className="file-read-code" ariaLabel={`Raw result for ${path}`} /></details>{notice && <p role="status">{notice}</p>}</div></details>;
}
