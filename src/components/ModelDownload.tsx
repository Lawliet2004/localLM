import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelDownloadInfo, type ModelInstallStatus } from '../lib/api';
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
export function ModelDownload({ busy, onSelect }: { busy: boolean; onSelect: (path: string) => void }) {
  const [info, setInfo] = useState<ModelDownloadInfo | null>(null);
  const [status, setStatus] = useState<ModelInstallStatus | null>(null);
  const [error, setError] = useState('');
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [revision, setRevision] = useState(0);
  const actionVersion = useRef(0);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      const version = actionVersion.current;
      try {
        const [nextInfo, nextStatus] = await Promise.all([api.modelDownloadInfo(), api.modelInstallStatus()]);
        if (!disposed && version === actionVersion.current) { setInfo(nextInfo); setStatus(nextStatus); }
      } catch (e) { if (!disposed && version === actionVersion.current) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 1000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [revision]);
  async function install() {
    actionVersion.current += 1;
    setStarting(true); setError('');
    try { setStatus(await api.installModel()); }
    catch (e) { setError(errorMessage(e)); }
    finally { actionVersion.current += 1; setStarting(false); setRevision(value => value + 1); }
  }
  const active = starting || status?.busy;
  const insufficient = info && !info.destinationExists && info.availableBytes < info.requiredBytes;
  return <section className="catalog-notice" aria-label="Recommended model download">
    <h2>MiniCPM5-2B · Q6_K</h2>
    <p>Download the pinned GGUF from Hugging Face, or verify an existing managed copy. The file is checked against its SHA-256 before use.</p>
    {info && <><p>{gib(info.bytes)} download · {gib(info.availableBytes)} free · {gib(info.requiredBytes)} required including reserve</p><details><summary>File details</summary><p className="download-path">{info.destination}</p><code className="download-path">SHA-256: {info.sha256}</code></details></>}
    {insufficient && <p role="alert">Not enough disk space for this download.</p>}
    {error && <p role="alert">{error}</p>}{status?.error && <p role="alert">{status.error}</p>}
    {active && <div role="status"><p>{status?.phase === 'verifying' ? 'Verifying existing model' : 'Downloading model'} · {gib(status?.received ?? 0)} / {gib(status?.total || info?.bytes || 0)}</p><progress aria-label="Model installation progress" max={status?.total || info?.bytes || 1} value={status?.received ?? 0} /></div>}
    {status?.phase === 'ready' && <p role="status">Model verified. Select it below, then save your model settings.</p>}
    <div className="connector-actions"><button type="button" className="secondary" disabled={!nativeAvailable || !info || active || Boolean(insufficient)} onClick={() => void install()}>{info?.destinationExists ? 'Verify managed model' : 'Download model'}</button>
      {active && <button type="button" className="secondary" disabled={cancelling} onClick={async () => { setCancelling(true); try { await api.cancelModelInstall(); } catch (e) { setError(errorMessage(e)); } finally { setCancelling(false); } }}>Cancel installation</button>}
      {status?.phase === 'ready' && status.path && <button type="button" className="primary" disabled={busy || Boolean(active)} onClick={() => onSelect(status.path!)}>Use verified model</button>}</div>
  </section>;
}
