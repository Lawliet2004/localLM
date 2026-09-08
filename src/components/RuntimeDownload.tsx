import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelInstallStatus } from '../lib/api';
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
export function RuntimeDownload({ busy, onSelect }: { busy: boolean; onSelect: (path: string) => void }) {
  const [info, setInfo] = useState<Awaited<ReturnType<typeof api.runtimeDownloadInfo>> | null>(null);
  const [status, setStatus] = useState<ModelInstallStatus | null>(null);
  const [error, setError] = useState('');
  const [starting, setStarting] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [revision, setRevision] = useState(0);
  const [installed, setInstalled] = useState<Awaited<ReturnType<typeof api.listInstalledRuntimes>>>([]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.listInstalledRuntimes().then(value => { if (!disposed) setInstalled(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [revision]);
  const version = useRef(0);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      const current = version.current;
      try {
        const [nextInfo, nextStatus] = await Promise.all([api.runtimeDownloadInfo(), api.runtimeInstallStatus()]);
        if (!disposed && current === version.current) { setInfo(nextInfo); setStatus(nextStatus); }
      } catch (e) { if (!disposed && current === version.current) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 1000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [revision]);
  async function install() {
    version.current++; setStarting(true); setError('');
    try { setStatus(await api.installRuntime()); }
    catch (e) { setError(errorMessage(e)); }
    finally { version.current++; setStarting(false); setRevision(value => value + 1); }
  }
  const active = Boolean(starting || status?.busy);
  const insufficient = info && info.availableBytes < info.requiredBytes;
  return <section className="catalog-notice" aria-label="CUDA runtime download">
    <h2>llama.cpp · CUDA 12.4</h2><p>Install the pinned b10855 Windows runtime and CUDA libraries. Each installation uses a new directory; existing runtimes are preserved.</p>
    {info && <p>{gib(info.bytes)} download · {gib(info.requiredBytes)} required for download, extraction and reserve · {gib(info.availableBytes)} free</p>}
    {insufficient && !active && <p role="alert">Not enough disk space to install the runtime.</p>}
    {error && <p role="alert">{error}</p>}{status?.error && <p role="alert">{status.error}</p>}
    {active && <div role="status"><p>{status?.phase === 'extracting' ? 'Verifying and extracting runtime files…' : 'Downloading runtime archive…'}</p>{status?.phase !== 'extracting' && <progress aria-label="Runtime archive download progress" max={status?.total || 1} value={status?.received || 0} />}</div>}
    <div className="connector-actions"><button type="button" className="secondary" disabled={!nativeAvailable || !info || active || Boolean(insufficient)} onClick={() => void install()}>Install CUDA runtime</button>
      {active && <button type="button" className="secondary" disabled={cancelling} onClick={async () => { setCancelling(true); try { await api.cancelRuntimeInstall(); } catch (e) { setError(errorMessage(e)); } finally { setCancelling(false); } }}>Cancel runtime installation</button>}
      {!active && status?.phase === 'ready' && status.path && <button type="button" className="primary" disabled={busy} onClick={() => onSelect(status.path!)}>Use installed runtime</button>}</div>
    {!active && status?.phase === 'ready' && <p role="status">Runtime installed. Select it, then save settings to use it.</p>}
    {installed.length > 0 && <details><summary>Installed runtimes ({installed.length})</summary><p>Checks file presence and sizes; does not recheck file hashes.</p>{installed.map(item => <div key={item.id}><p className="download-path">{item.id}</p>{item.problem && <p>{item.problem}</p>}<button type="button" className="secondary" disabled={busy || active || !item.complete} onClick={() => onSelect(item.path)}>Select {item.id.slice(-8)}</button></div>)}</details>}
  </section>;
}
