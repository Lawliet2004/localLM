import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { refreshDownloadJobs, useDownloadJob } from '../lib/downloadJobs';
import { JobProgress } from './JobProgress';
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
export interface RuntimeDownloadBackend {
  info: typeof api.runtimeDownloadInfo;
  install: typeof api.installRuntime;
  cancel: typeof api.cancelRuntimeInstall;
  list?: typeof api.listInstalledRuntimes;
}
export function RuntimeDownload({ busy, onSelect, backend, title = 'llama.cpp · CUDA 12.4', description = 'Install the pinned b10855 Windows runtime and CUDA libraries. An installed pinned runtime is reused instead of downloading again.', selectable = true, jobKey = 'runtime:standard' }: { busy: boolean; onSelect: (path: string) => void; backend?: RuntimeDownloadBackend; title?: string; description?: string; selectable?: boolean; jobKey?: string }) {
  const [info, setInfo] = useState<Awaited<ReturnType<typeof api.runtimeDownloadInfo>> | null>(null);
  const [infoError, setInfoError] = useState('');
  const [error, setError] = useState('');
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [revision, setRevision] = useState(0);
  const [installed, setInstalled] = useState<Awaited<ReturnType<typeof api.listInstalledRuntimes>>>([]);
  const job = useDownloadJob(jobKey);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    (backend?.list ? backend.list() : api.listInstalledRuntimes()).then(value => { if (!disposed) setInstalled(value); }).catch(() => {});
    return () => { disposed = true; };
  }, [revision, backend]);
  // Job status comes from the shared store; info is this panel's own concern.
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const next = await (backend ? backend.info() : api.runtimeDownloadInfo());
        if (!disposed) { setInfo(next); setInfoError(''); }
      } catch (e) { if (!disposed) setInfoError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 5000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [revision, backend]);
  async function install() {
    setStarting(true); setError('');
    try { await (backend ? backend.install() : api.installRuntime()); }
    catch (e) { setError(errorMessage(e)); }
    finally { setStarting(false); setRevision(value => value + 1); refreshDownloadJobs(); }
  }
  const active = Boolean(starting || job?.busy);
  const insufficient = info && info.requiredBytes > 0 && info.availableBytes < info.requiredBytes;
  const primary = info?.reusable ? 'Use installed runtime'
    : (info && info.savedBytes > 0) || job?.phase === 'cancelled' || job?.phase === 'interrupted' ? 'Resume runtime download'
    : 'Install CUDA runtime';
  const showJob = job && (job.busy || ['failed', 'cancelled', 'interrupted', 'ready'].includes(job.phase));
  const ready = job?.phase === 'ready' && job.path;
  return <section className="catalog-notice" aria-label="CUDA runtime download">
    <h2>{title}</h2><p>{description}</p>
    {info && <p>{gib(Math.max(0, info.bytes - info.savedBytes))} to download · {gib(info.savedBytes)} saved · {gib(info.requiredBytes)} required incl. reserve · {gib(info.availableBytes)} free</p>}
    {insufficient && !active && <p role="alert">Not enough disk space to install the runtime.</p>}
    {infoError && <p role="alert">{infoError}</p>}{error && <p role="alert">{error}</p>}
    {showJob && job && <JobProgress job={job} />}
    <div className="connector-actions"><button type="button" className="secondary" disabled={!nativeAvailable || !info || active || Boolean(insufficient)} onClick={() => void install()}>{primary}</button>
      {job?.busy && <button type="button" className="secondary" disabled={cancelling} onClick={async () => { setCancelling(true); try { await (backend ? backend.cancel(job.jobId) : api.cancelRuntimeInstall(job.jobId)); } catch (e) { setError(errorMessage(e)); } finally { setCancelling(false); refreshDownloadJobs(); } }}>Pause download</button>}
      {selectable && ready && <button type="button" className="primary" disabled={busy} onClick={() => onSelect(job.path!)}>Use installed runtime</button>}</div>
    {ready && <p role="status">{selectable ? 'Runtime installed. Select it, then save settings to use it.' : 'Runtime installed. Verify the text weights, then use the model.'}</p>}
    {installed.length > 0 && <details><summary>Installed runtimes ({installed.length})</summary><p>Checks file presence and sizes; does not recheck file hashes.</p>{installed.map(item => <div key={item.id}><p className="download-path">{item.id}</p>{item.problem && <p>{item.problem}</p>}{selectable && <button type="button" className="secondary" disabled={busy || active || !item.complete} onClick={() => onSelect(item.path)}>Select {item.id.slice(-8)}</button>}</div>)}</details>}
  </section>;
}
