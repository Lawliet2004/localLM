import { save } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Copy, Download, ExternalLink } from 'lucide-react';
import { MarkdownView } from './MarkdownView';
import { CodeBlock } from './CodeBlock';
import { languageFromPath } from '../lib/highlight';
export function parseDelimited(text: string, delimiter = ','): string[][] {
  const rows: string[][] = []; let row: string[] = [], cell = '', quoted = false;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (char === '"') { if (quoted && text[i + 1] === '"') { cell += '"'; i++; } else quoted = !quoted; }
    else if (char === delimiter && !quoted) { row.push(cell); cell = ''; }
    else if (char === '\n' && !quoted) { row.push(cell.replace(/\r$/, '')); rows.push(row); row = []; cell = ''; if (rows.length >= 101) break; }
    else cell += char;
  }
  if ((cell || row.length) && rows.length < 101) rows.push([...row, cell]);
  return rows;
}
/** Never executes generated HTML/SVG/scripts. Images use the native verified bytes. */
export function ArtifactViewer({ name, content, mimeType, bytes, onClose }: { name: string; content: string; mimeType: string; bytes?: number; onClose?: () => void }) {
  const [mode, setMode] = useState<'preview' | 'source'>('preview');
  const [notice, setNotice] = useState('');
  const [openUrl, setOpenUrl] = useState('');
  const expanded = useRef<HTMLDialogElement>(null);
  const [expandedOpen, setExpandedOpen] = useState(false);
  useEffect(() => { if (expandedOpen) expanded.current?.showModal(); }, [expandedOpen]);
  const image = /^image\/(png|jpeg|webp)$/.test(mimeType) && /^data:image\/(png|jpeg|webp);base64,/.test(content);
  const markdown = /\.md$/i.test(name);
  const delimiter = /\.tsv$/i.test(name) ? '\t' : ',';
  const table = /\.(csv|tsv)$/i.test(name);
  const rows = useMemo(() => table ? parseDelimited(content, delimiter) : [], [table, content, delimiter]);
  useEffect(() => {
    if (typeof URL.createObjectURL !== 'function') return;
    let blob: Blob;
    try {
      blob = image ? new Blob([Uint8Array.from(atob(content.split(',')[1]), character => character.charCodeAt(0))], { type: mimeType }) : new Blob([content], { type: 'text/plain;charset=utf-8' });
    } catch { setNotice('This image could not be opened.'); return; }
    const url = URL.createObjectURL(blob); setOpenUrl(url);
    return () => { URL.revokeObjectURL(url); };
  }, [content, image, mimeType]);
  const href = image ? content : `data:text/plain;charset=utf-8,${encodeURIComponent(content)}`;
  return <section className="artifact-viewer" aria-label={`Artifact ${name}`}><header><strong>{name}</strong><small>{mimeType}{bytes === undefined ? '' : ` · ${bytes.toLocaleString()} bytes`}</small></header>
    <div className="artifact-actions"><div role="group" aria-label={`${name} view mode`}><button className="secondary" aria-pressed={mode === 'preview'} onClick={() => setMode('preview')}>Preview</button><button className="secondary" aria-pressed={mode === 'source'} onClick={() => setMode('source')}>Source</button></div><button className="secondary" aria-label={`Copy ${name} source`} onClick={async () => { try { await navigator.clipboard.writeText(content); setNotice('Copied source.'); } catch { setNotice('Copy failed. Select the source text to copy it.'); } }}><Copy size={14} />Copy</button>{nativeAvailable ? <button className="secondary" onClick={async () => { try { const path = await save({ title: 'Save artifact copy', defaultPath: name.split(/[\\/]/).pop() }); if (!path) return; await api.saveArtifactCopy(path, content, mimeType); setNotice(`Saved ${path}`); } catch (error) { setNotice(errorMessage(error)); } }}><Download size={14} />Save</button> : <a className="secondary" download={name.split(/[\\/]/).pop()} href={href}><Download size={14} />Save</a>}{nativeAvailable ? <button className="secondary" onClick={() => setExpandedOpen(true)}><ExternalLink size={14} />Open</button> : <a className="secondary" href={openUrl || undefined} aria-disabled={!openUrl} target="_blank" rel="noreferrer"><ExternalLink size={14} />Open</a>}{onClose && <button className="secondary" aria-label={`Close ${name} preview`} onClick={onClose}>Close preview</button>}</div>
    {mode === 'preview' && image ? <img src={content} alt={name} /> : mode === 'preview' && table && rows.length ? <div className="artifact-table" tabIndex={0} role="region" aria-label={`${name} table`}><table><thead><tr>{rows[0].map((cell, i) => <th key={i} scope="col">{cell}</th>)}</tr></thead><tbody>{rows.slice(1, 101).map((row, i) => <tr key={i}>{row.map((cell, j) => <td key={j}>{cell}</td>)}</tr>)}</tbody></table>{rows.length > 100 && <p>Preview shows the first 100 rows. Source and Save include the full file.</p>}</div> : mode === 'preview' && markdown ? <MarkdownView content={content} /> : <CodeBlock code={content} language={languageFromPath(name)} defaultWrap ariaLabel={`${name} source`} />}
    <dialog ref={expanded} className="artifact-expanded" aria-label={`Expanded ${name}`} onClose={() => setExpandedOpen(false)}>{expandedOpen && <><header><strong>{name}</strong><button className="secondary" autoFocus onClick={() => expanded.current?.close()}>Close expanded preview</button></header>{image ? <img src={content} alt={name} /> : <pre tabIndex={0}>{content}</pre>}</>}</dialog>
    {/\.html?$/i.test(name) && <p>HTML is shown as source. Preview does not execute scripts.</p>}
    {notice && <p role="status">{notice}</p>}
  </section>;
}
