import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
export function ProjectDocuments({ root, busy }: { root?: string; busy: boolean }) {
  const [enabled, setEnabled] = useState(false);
  const [paths, setPaths] = useState('');
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<{ path: string; line: number; excerpt: string }[]>([]);
  const [notice, setNotice] = useState('');
  const [pending, setPending] = useState(false);
  useEffect(() => {
    setEnabled(false); setPaths(''); setHits([]); setNotice('');
    if (!nativeAvailable || !root || !api.documentIndexStatus) return;
    let disposed = false;
    api.documentIndexStatus(root).then(value => { if (!disposed) { setEnabled(value.enabled); setPaths(value.paths.join('\n')); } }).catch(error => { if (!disposed) setNotice(errorMessage(error)); });
    return () => { disposed = true; };
  }, [root]);
  async function save(next: boolean) {
    if (!root) return;
    setPending(true); setNotice('');
    try { const value = await api.saveDocumentIndex(root, next, paths.split('\n').map(path => path.trim()).filter(Boolean)); setEnabled(value.enabled); setPaths(value.paths.join('\n')); setHits([]); setNotice(next ? 'Project documents enabled. Current file contents are read for each search and response.' : 'Project document selection deleted. Retrieval is off.'); }
    catch (error) { setNotice(errorMessage(error)); } finally { setPending(false); }
  }
  return <section className="preference-section project-documents" aria-labelledby="project-documents-title"><h2 id="project-documents-title">Project documents</h2>
    <div className="preference-group"><div className="preference-row preference-row-block"><div className="preference-copy"><label>Local document retrieval</label><p>{root || 'Choose a project folder first.'} · {enabled ? 'Enabled' : 'Off'}</p><details className="project-documents-learn"><summary>Learn more</summary><p>Opt in to local keyword retrieval for this project. Choose up to 16 UTF-8 Markdown, text, RST, CSV or TSV files, at most 32 KiB each. No background scan or embedding service runs. Selected excerpts enter your prompt and go to the API provider when an API model is selected.</p></details></div></div></div>
    <label>Document paths, one per line<textarea rows={4} value={paths} disabled={!root || busy || pending || !nativeAvailable} onChange={event => setPaths(event.target.value)} placeholder={'docs/overview.md\nREADME.md'} /></label>
    <div className="field-row"><button className="primary" disabled={!root || !paths.trim() || busy || pending || !nativeAvailable} onClick={() => void save(true)}>{pending ? 'Saving…' : enabled ? 'Update document selection' : 'Enable project documents'}</button><button className="secondary" disabled={!enabled || busy || pending} onClick={() => void save(false)}>Disable and delete selection</button></div>
    {enabled && <form onSubmit={async event => { event.preventDefault(); if (!root) return; setPending(true); try { const results = await api.searchDocumentIndex(root, query); setHits(results); setNotice(results.length ? `${results.length} matching excerpts` : 'No matching current documents. Deleted or inaccessible files are excluded.'); } catch (error) { setNotice(errorMessage(error)); } finally { setPending(false); } }}><label>Test document retrieval<input value={query} onChange={event => setQuery(event.target.value)} maxLength={4096} /></label><button className="secondary" disabled={pending || !query.trim()}>Search documents</button></form>}
    {notice && <p role="status">{notice}</p>}
    {hits.map(hit => <details key={`${hit.path}:${hit.line}`}><summary>{hit.path}:{hit.line}</summary><pre>{hit.excerpt}</pre></details>)}
  </section>;
}
