import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelDownloadInfo } from '../lib/api';
import { localModels } from '../lib/localModels';
import { refreshDownloadJobs, useDownloadJob } from '../lib/downloadJobs';
import { JobProgress } from './JobProgress';
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
export interface ModelDownloadBackend {
  info: (filename: string) => Promise<ModelDownloadInfo>;
  install: (filename: string) => Promise<Awaited<ReturnType<typeof api.installModel>>>;
  cancel: (jobId?: number) => Promise<void>;
  list?: () => Promise<unknown>;
}
export function ModelDownload({ busy, onSelect, filename = localModels[0].filename, backend, label, verifiedMessage, jobKey }: { busy: boolean; onSelect: (path: string) => void; filename?: string; backend?: ModelDownloadBackend; label?: string; verifiedMessage?: string; jobKey?: string }) {
  const key = jobKey ?? `model:catalog:${filename}`;
  const [info, setInfo] = useState<ModelDownloadInfo | null>(null);
  const [infoError, setInfoError] = useState('');
  const [error, setError] = useState('');
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [revision, setRevision] = useState(0);
  const job = useDownloadJob(key);
  // Job status comes from the shared store; info is this panel's own concern.
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const next = await (backend ? backend.info(filename) : api.modelDownloadInfo(filename));
        if (!disposed) { setInfo(next); setInfoError(''); }
      } catch (e) { if (!disposed) setInfoError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 5000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [revision, filename, backend]);
  async function install() {
    setStarting(true); setError('');
    try { await (backend ? backend.install(filename) : api.installModel(filename)); }
    catch (e) { setError(errorMessage(e)); }
    finally { setStarting(false); setRevision(value => value + 1); refreshDownloadJobs(); }
  }
  const active = Boolean(starting || job?.busy);
  const verified = job?.phase === 'ready' && job.path === info?.destination;
  const insufficient = info && info.requiredBytes > 0 && info.availableBytes < info.requiredBytes;
  const primary = info?.destinationExists ? 'Verify managed model'
    : (info && info.savedBytes > 0) || job?.phase === 'cancelled' || job?.phase === 'interrupted' ? 'Resume download'
    : 'Download model';
  const showJob = job && (job.busy || ['failed', 'cancelled', 'interrupted', 'ready'].includes(job.phase));
  return <section className="catalog-notice" aria-label={label ? `${label} download` : 'Recommended model download'}>
    <h2>{label ?? localModels.find(model => model.filename === filename)?.label ?? filename}</h2>
    <p>Download the pinned GGUF from Hugging Face, or verify an existing managed copy. The file is checked against its SHA-256 before use.</p>
    {info && <><p>{gib(info.remainingBytes)} to download · {gib(info.savedBytes)} saved · {gib(info.requiredBytes)} required incl. reserve · {gib(info.availableBytes)} free</p><details><summary>File details</summary><p className="download-path">{info.destination}</p><code className="download-path">SHA-256: {info.sha256}</code></details></>}
    {insufficient && <p role="alert">Not enough disk space for this download.</p>}
    {infoError && <p role="alert">{infoError}</p>}{error && <p role="alert">{error}</p>}
    {showJob && job && <JobProgress job={job} />}
    {verified && <p role="status">{verifiedMessage ?? 'Model verified. Select it below, then save your model settings.'}</p>}
    <div className="connector-actions"><button type="button" className="secondary" disabled={!nativeAvailable || !info || active || Boolean(insufficient)} onClick={() => void install()}>{primary}</button>
      {job?.busy && <button type="button" className="secondary" disabled={cancelling} onClick={async () => { setCancelling(true); try { await (backend ? backend.cancel(job.jobId) : api.cancelModelInstall(job.jobId)); } catch (e) { setError(errorMessage(e)); } finally { setCancelling(false); refreshDownloadJobs(); } }}>Pause download</button>}
      {verified && job?.path && <button type="button" className="primary" disabled={busy || Boolean(active)} onClick={() => onSelect(job.path!)}>Use verified model</button>}</div>
  </section>;
}
