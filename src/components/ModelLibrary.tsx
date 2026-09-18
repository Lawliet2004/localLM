import { useEffect, useMemo, useState } from 'react';
import { Download, HardDrive, Heart, Search, Trash2, Play, RefreshCw, ExternalLink } from 'lucide-react';
import { api, errorMessage, nativeAvailable, type ModelInstallStatus } from '../lib/api';
import type { HardwareStatus, HubRepo, HubSearchHit, InstalledModel } from '../lib/types';
import { modelLabel } from '../lib/localModels';
import { compactCount, downloadSize, fitKind, fitLabel, parseHubQuery, preferredFile, preferredProjector, projectorFiles, quantLabel, repoParts, weightFiles } from '../lib/modelStudio';
import { gib } from '../lib/runtimeGuidance';
import { samePath } from '../lib/pathUtils';

function downloadedBytes(model: InstalledModel) {
  return model.received ?? (model.complete ? model.bytes : 0);
}
function modelStatus(model: InstalledModel, selectedPath: string, loadedPath: string | null) {
  if (!model.complete) return downloadedBytes(model) > 0 ? 'Interrupted' : 'Incomplete';
  if (samePath(loadedPath, model.path)) return 'Running';
  if (samePath(selectedPath, model.path)) return 'Selected';
  return null;
}

interface Props {
  models: InstalledModel[]; loading: boolean; busy: boolean; selectedPath: string; loadedPath: string | null;
  onRefresh: () => Promise<void>; onUse: (model: InstalledModel) => Promise<void>; onDeleted: () => Promise<void>;
}
export function ModelLibrary({ models, loading, busy, selectedPath, loadedPath, onRefresh, onUse, onDeleted }: Props) {
  const [view, setView] = useState<'discover' | 'library'>('discover');
  const [query, setQuery] = useState('');
  const [libraryQuery, setLibraryQuery] = useState('');
  const [token, setToken] = useState('');
  const [results, setResults] = useState<HubSearchHit[]>([]);
  const [catalog, setCatalog] = useState<HubRepo | null>(null);
  const [filename, setFilename] = useState('');
  const [projector, setProjector] = useState('');
  const [working, setWorking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [status, setStatus] = useState<ModelInstallStatus | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [remove, setRemove] = useState<string | null>(null);
  const [searched, setSearched] = useState(false);
  const [browseMode, setBrowseMode] = useState<'popular' | 'search'>('popular');
  const [hardware, setHardware] = useState<HardwareStatus | null>(null);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    let wasBusy = false;
    async function poll() {
      try {
        const value = await api.modelInstallStatus();
        if (!disposed) {
          setStatus(value);
          if (wasBusy && !value.busy) await onRefresh();
          wasBusy = value.busy;
        }
      } catch (e) { if (!disposed) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void poll(), 1000); }
    }
    void poll();
    void api.hardwareStatus().then(value => { if (!disposed) setHardware(value); }).catch(() => {});
    return () => { disposed = true; clearTimeout(timer); };
  }, [onRefresh]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    void api.searchHuggingFace('').then(value => {
      if (!disposed) { setResults(value); setSearched(true); setBrowseMode('popular'); }
    }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  const locked = busy || working;
  const downloadLocked = locked || downloading || Boolean(status?.busy);
  async function download(repo: string, revision: string, file: string, projectorFile?: string) {
    setDownloading(true); setError(''); setNotice('');
    try {
      const result = await api.downloadHuggingFaceModel(repo, revision, file, token || undefined, projectorFile || null);
      setStatus(result); await onRefresh();
      if (result.error) throw new Error(result.error);
      setNotice(projectorFile ? 'Model and vision projector verified. Choose Use model in your library.' : 'Download verified. Choose Use model in your library.');
    } catch (e) { setError(errorMessage(e)); }
    finally { setDownloading(false); }
  }
  async function run(action: () => Promise<void>) {
    setWorking(true); setError(''); setNotice('');
    try { await action(); } catch (e) { setError(errorMessage(e)); }
    finally { setWorking(false); }
  }
  async function openRepo(value: string) {
    setCatalog(null); setFilename(''); setProjector('');
    const result = await api.huggingFaceFiles(value.trim(), token || undefined);
    setCatalog(result);
    const weights = preferredFile(result.files);
    setFilename(weights);
    setProjector(preferredProjector(result.files, weights));
    setView('discover');
  }
  async function searchHub(raw: string) {
    const repo = parseHubQuery(raw);
    if (repo) {
      setBrowseMode('search');
      setSearched(true);
      await openRepo(repo);
      return;
    }
    setCatalog(null);
    setFilename('');
    setProjector('');
    setResults(await api.searchHuggingFace(raw, token || undefined));
    setSearched(true);
    setBrowseMode(raw.trim() ? 'search' : 'popular');
  }
  const quantRows = useMemo(() => catalog ? weightFiles(catalog.files).map(file => {
    const bytes = downloadSize(catalog.files, file.filename) ?? file.bytes;
    const kind = fitKind(bytes, hardware);
    return { file, bytes, kind, quant: quantLabel(file.filename) };
  }) : [], [catalog, hardware]);
  const selected = quantRows.find(row => row.file.filename === filename);
  const projectors = useMemo(() => catalog ? projectorFiles(catalog.files) : [], [catalog]);
  const projectorBytes = useMemo(() => {
    if (!catalog || !projector) return 0;
    return downloadSize(catalog.files, projector) ?? catalog.files.find(file => file.filename === projector)?.bytes ?? 0;
  }, [catalog, projector]);
  const visibleModels = models.filter(model => {
    const haystack = `${modelLabel(model.filename)} ${model.filename} ${model.repo ?? ''}`.toLowerCase();
    return haystack.includes(libraryQuery.trim().toLowerCase());
  });
  return <div className="studio">
    <div className="studio-switch" role="tablist" aria-label="Model library">
      <button type="button" role="tab" aria-selected={view === 'discover'} onClick={() => setView('discover')}><Search size={14} />Discover</button>
      <button type="button" role="tab" aria-label={`My Models, ${models.length}`} aria-selected={view === 'library'} onClick={() => setView('library')}><HardDrive size={14} />My Models<span className="studio-count">{models.length}</span></button>
    </div>
    {view === 'discover' ? <div className="studio-discover">
      <section className="studio-pane" aria-labelledby="discover-heading">
        <div className="studio-pane-head">
          <div><p className="eyebrow">HUGGING FACE</p><h2 id="discover-heading">Discover</h2></div>
        </div>
        <p className="section-description">Search GGUF models, paste an owner/name, or drop a Hugging Face URL. Choose a quantization that fits this machine, then download.</p>
        <form className="studio-search" onSubmit={e => { e.preventDefault(); void run(() => searchHub(query)); }}>
          <label className="studio-search-field">Search Hugging Face
            <span><Search size={15} /><input placeholder="qwen, bartowski/Llama-3.2-3B-Instruct-GGUF, or a Hugging Face URL" value={query} onChange={e => setQuery(e.target.value)} maxLength={300} /></span>
          </label>
          <button className="primary" disabled={locked}>Search</button>
        </form>
        <details className="hub-access"><summary>Private or gated repository</summary><label>Hugging Face read token<input type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} placeholder="hf_…" /></label><small>Used only for this page session. Accept any required license on Hugging Face first.</small></details>
        {searched && !results.length && !catalog && <p>No matching repositories. Try another name or paste an owner/model.</p>}
        {results.length > 0 && <div className="studio-results" aria-label={browseMode === 'popular' ? 'Popular GGUF models' : 'Hugging Face search results'}>
          <p className="hub-results-heading">{browseMode === 'popular' ? 'Popular GGUF models' : 'Search results'}</p>
          {results.map(hit => {
            const parts = repoParts(hit.id);
            return <button type="button" className={`studio-hit${catalog?.repo === hit.id ? ' selected' : ''}`} key={hit.id} disabled={locked} onClick={() => void run(() => openRepo(hit.id))}>
              <span className="studio-avatar" aria-hidden>{parts.publisher.slice(0, 1).toUpperCase()}</span>
              <span className="studio-hit-copy"><strong>{parts.name}</strong><small>{hit.id}</small></span>
              <span className="studio-hit-meta">{hit.downloads != null && <span>{compactCount(hit.downloads)} downloads</span>}{hit.likes != null && <span><Heart size={11} /> {compactCount(hit.likes)}</span>}</span>
            </button>;
          })}
        </div>}
      </section>
      <section className="studio-pane studio-detail" aria-labelledby="quant-heading">
        {!catalog && <div className="studio-detail-empty"><Download size={28} /><h3 id="quant-heading">Select a model</h3><p>Pick a repository on the left to see every GGUF quantization, its size, and whether it is likely to fit this GPU or system RAM.</p></div>}
        {catalog && <>
          <div className="studio-pane-head">
            <div><p className="eyebrow">{repoParts(catalog.repo).publisher}</p><h2 id="quant-heading">{repoParts(catalog.repo).name}</h2></div>
            <a className="secondary" href={`https://huggingface.co/${catalog.repo}`} target="_blank" rel="noreferrer"><ExternalLink size={14} />Hugging Face</a>
          </div>
          <p className="section-description">{catalog.repo} · SHA-256 verified · Split models download every shard together. Q4_K_M is the usual starting point when it is available.</p>
          {!quantRows.length ? <p>No downloadable GGUF weight files with verification metadata found in this repository.</p> : <div className="quant-table" role="table" aria-label="GGUF file / quantization">
            <div className="quant-row quant-head" role="row"><div className="quant-select"><span>Quant</span><span>File</span><span>Size</span><span>This machine</span></div><span /></div>
            {quantRows.map(row => <div className={`quant-row${filename === row.file.filename ? ' selected' : ''}`} key={row.file.filename} role="row">
              <button type="button" className="quant-select" disabled={locked} aria-pressed={filename === row.file.filename} onClick={() => { setFilename(row.file.filename); setProjector(current => current || preferredProjector(catalog?.files ?? [], row.file.filename)); }}>
                <strong>{row.quant}</strong>
                <span className="quant-name">{row.file.filename.split('/').pop()}</span>
                <span>{gib(row.bytes)}</span>
                <span className={`fit-badge fit-${row.kind}`}>{fitLabel(row.kind)}</span>
              </button>
              <button type="button" className="secondary" aria-label={`Download ${row.quant}`} disabled={downloadLocked} onClick={() => { setFilename(row.file.filename); void download(catalog.repo, catalog.revision, row.file.filename, projector || undefined); }}>Download</button>
            </div>)}
          </div>}
          {selected && projectors.length > 0 && <div className="quant-projector">
            <label>Vision projector (optional, enables image input)
              <select aria-label="Vision projector" value={projector} disabled={locked} onChange={e => setProjector(e.target.value)}>
                <option value="">None · text only</option>
                {projectors.map(file => <option key={file.filename} value={file.filename}>{file.filename.split('/').pop()} · {gib(file.bytes)}</option>)}
              </select>
              <small>Multimodal models need their matching mmproj file to see images. {projector ? `${gib(projectorBytes)} downloads with the model.` : 'Leave empty for text-only chat.'}</small>
            </label>
          </div>}
          {selected && <div className="quant-cta"><div><strong>{selected.quant}</strong><p>{gib(selected.bytes)}{projector ? ` + ${gib(projectorBytes)} projector` : ''} · {fitLabel(selected.kind)}. Projectors and adapters are companion files, not standalone chat models.</p></div><button type="button" className="primary" disabled={downloadLocked || !filename} onClick={() => void download(catalog.repo, catalog.revision, filename, projector || undefined)}><Download size={14} />Download model{projector ? ' + projector' : ''}</button></div>}
        </>}

      </section>
    </div> : <section className="studio-pane studio-library" aria-labelledby="library-heading">
      <div className="studio-pane-head">
        <div><p className="eyebrow">ON THIS MACHINE</p><h2 id="library-heading">My Models <span className="selection-badge">{models.length}</span></h2></div>
        <button type="button" className="secondary" disabled={locked || loading} onClick={() => void run(onRefresh)}><RefreshCw size={14} />Refresh</button>
      </div>
      <p className="section-description">Load any downloaded GGUF, including the original catalog models. Each file remembers its runtime settings. Deleting a model keeps your conversations.</p>
      {models.length > 0 && <label className="studio-search-field">Filter downloaded models<input placeholder="Filter by name or publisher" value={libraryQuery} onChange={e => setLibraryQuery(e.target.value)} /></label>}
      {loading && <p>Reading your model library…</p>}
      {!loading && !models.length && <div className="library-empty"><HardDrive size={24} /><h3>No models downloaded yet</h3><p>Open Discover, pick a GGUF quantization that fits, then download it.</p><button type="button" className="primary" onClick={() => setView('discover')}>Discover models</button></div>}
      <div className="library-list">{visibleModels.map(model => {
        const activeDownload = status?.busy && samePath(status.path, model.path.replace(/\.part$/, ''));
        const received = activeDownload ? status.received : downloadedBytes(model);
        const statusLabel = activeDownload ? (status.phase === 'verifying' ? 'Verifying' : 'Downloading') : modelStatus(model, selectedPath, loadedPath);
        const quant = quantLabel(model.filename);
        return <article className={`library-model${statusLabel === 'Running' ? ' running' : ''}`} key={model.id}>
          <span className="studio-avatar" aria-hidden>{(model.repo?.split('/')[0] ?? 'G').slice(0, 1).toUpperCase()}</span>
          <div className="library-model-info">
            <strong>{modelLabel(model.filename)}</strong>
            <span>{model.repo ? `Hugging Face · ${model.repo}` : 'Local GGUF file'}</span>
            <small>{quant} · {model.complete ? gib(model.bytes) : `${gib(received)} of ${gib(model.bytes)} saved`} · {model.files.length > 1 ? `${model.files.length} files` : '1 file'}{model.projectorFilename ? ` · Vision: ${model.projectorFilename}` : ''}</small>
            {statusLabel && <span className={`library-chip${statusLabel === 'Incomplete' || statusLabel === 'Interrupted' ? ' warning' : ''}`}>{statusLabel}</span>}
            {!model.complete && <progress aria-label={`${modelLabel(model.filename)} download progress`} value={received} max={model.bytes || 1} />}
            <details><summary>File location & source</summary>{model.files.map(path => <p key={path}>{path}</p>)}{model.repo && <a href={`https://huggingface.co/${model.repo}`} target="_blank" rel="noreferrer">View on Hugging Face ↗</a>}{model.revision && <small>Revision {model.revision}</small>}</details>
          </div>
          <div className="library-actions">
            <button className="primary" type="button" disabled={locked || !model.complete} onClick={() => void run(async () => { await onUse(model); setNotice(`${modelLabel(model.filename)} is ready to use.`); })}><Play size={14} />Use model</button>
            {!model.complete && <button type="button" className="secondary" disabled={downloadLocked} onClick={() => {
              if (model.repo && model.revision) void download(model.repo, model.revision, model.filename);
              else if (model.repo) void run(() => openRepo(model.repo!));
              else { setNotice('Enter the Hugging Face repository in Discover to finish this download.'); setView('discover'); }
            }}><Download size={14} />{activeDownload ? 'Downloading…' : downloadedBytes(model) > 0 ? 'Resume download' : 'Retry download'}</button>}
            <button type="button" className="secondary" aria-label={`Delete ${modelLabel(model.filename)}`} disabled={downloadLocked} onClick={() => setRemove(model.id)}><Trash2 size={14} />Delete</button>
          </div>
          {remove === model.id && <div className="library-delete" role="group" aria-label="Confirm model deletion"><p>Delete {modelLabel(model.filename)} ({gib(model.bytes)}) from disk? Every listed copy will be removed. A running copy will be unloaded. Conversations are kept.</p><button type="button" className="secondary" disabled={locked} onClick={() => setRemove(null)}>Keep model</button><button type="button" className="primary" disabled={downloadLocked} onClick={() => void run(async () => { await api.deleteInstalledModel(model.id); setRemove(null); await onDeleted(); await onRefresh(); setNotice('Model deleted.'); })}>Delete from disk</button></div>}
        </article>;
      })}</div>
    </section>}
    {error && <p className="error" role="alert">{error}</p>}
    {notice && <p className="success" role="status">{notice}</p>}
  </div>;
}
