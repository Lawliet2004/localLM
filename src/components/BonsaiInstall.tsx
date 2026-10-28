import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelInstallStatus } from '../lib/api';
import { gib } from '../lib/runtimeGuidance';

type Choice = { id: string; backend: string; build: string };
export function BonsaiInstall({ busy, onInstalled }: { busy: boolean; onInstalled: (loaded: boolean) => Promise<void> }) {
  const [choices, setChoices] = useState<Choice[]>([]);
  const [runtimeId, setRuntimeId] = useState('');
  const [info, setInfo] = useState<Awaited<ReturnType<typeof api.runtimeDownloadInfo>> | null>(null);
  const [installing, setInstalling] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [status, setStatus] = useState<{ model: ModelInstallStatus; runtime: ModelInstallStatus } | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    if (nativeAvailable) void Promise.all([api.prismRuntimeChoices(), api.hardwareStatus().catch(() => null)]).then(([available, hardware]) => {
      if (!mounted.current) return;
      setChoices(available);
      const nvidia = hardware?.gpus.some(gpu => /nvidia/i.test(`${gpu.name}`));
      setRuntimeId((nvidia ? available.find(choice => choice.backend.startsWith('CUDA')) : undefined)?.id ?? available[0]?.id ?? '');
    }).catch(e => { if (mounted.current) setError(errorMessage(e)); });
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    setInfo(null);
    if (!runtimeId) return;
    let disposed = false;
    void api.runtimeDownloadInfo(runtimeId).then(value => { if (!disposed) setInfo(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [runtimeId]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const [model, runtime] = await Promise.all([api.modelInstallStatus(), api.runtimeInstallStatus()]);
        if (!disposed) setStatus({ model, runtime });
      } catch (e) { if (!disposed) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void poll(), 1000); }
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  const owned = status?.model.profileId === 'bonsai-2-9b';
  const active = installing || Boolean(owned && (status?.model.busy || status?.runtime.busy));
  const otherInstall = !owned && Boolean(status?.model.busy || status?.runtime.busy);
  const insufficient = info ? info.availableBytes < info.requiredBytes + 2675410720 : false;
  const progress = status?.runtime.phase === 'ready' ? status.model : status?.runtime;
  async function install(mtpMode: 'auto' | 'off' = 'auto') {
    setInstalling(true); setError(''); setNotice('');
    try {
      await api.installBonsai(runtimeId, mtpMode);
      await onInstalled(true);
      if (mounted.current) setNotice('Bonsai is loaded. Prism and the model are saved in managed storage.');
    } catch (e) {
      if (mounted.current) setError(errorMessage(e));
      // Refresh even after startup failure: verified downloads remain usable.
      await onInstalled(false).catch(() => {});
    } finally { if (mounted.current) setInstalling(false); }
  }
  return <section className="catalog-notice" aria-labelledby="bonsai-install-heading">
    <h2 id="bonsai-install-heading">Bonsai 2 9B · PQ2_0 + MTP</h2>
    <p>Install the model with its built-in MTP head and the matching Prism runtime. Both downloads are SHA-256 verified before selection. Starts with 16,384 context tokens and text input; add the matching vision projector in Model files for images.</p>
    {!nativeAvailable ? <p>Open the desktop app to install and run Bonsai.</p> : <>
      <label htmlFor="bonsai-runtime">Prism backend</label>
      <select id="bonsai-runtime" value={runtimeId} disabled={busy || active || otherInstall} onChange={e => setRuntimeId(e.target.value)}>
        {!choices.length && <option value="">No managed package available</option>}
        {choices.map(choice => <option key={choice.id} value={choice.id}>{choice.backend} · {choice.build}</option>)}
      </select>
      {info && <p>{gib(2675410720)} model + {gib(info.bytes)} runtime download · {gib(info.requiredBytes + 2675410720)} required including extraction and reserve · {gib(info.availableBytes)} free</p>}
      {choices.some(choice => choice.id === runtimeId && choice.backend.startsWith('CUDA')) && <p>CUDA requires a compatible NVIDIA driver. Choose CPU if CUDA cannot start.</p>}
      {info && <details><summary>Storage and compatibility</summary><p className="download-path">{info.destination}</p><p>Model weights are stored in the adjacent models folder. Runtime versions are installed side by side. Requires Prism b10826-c9c34bc; stock llama.cpp cannot load PQ2_0. MTP can be turned off in Runtime settings.</p></details>}
      {insufficient && <p className="error" role="alert">Not enough disk space to install Bonsai and Prism.</p>}
      {otherInstall && <p role="status">Another model or runtime installation is running. Wait for it to finish before installing Bonsai.</p>}
      {active && <div role="status"><p>{installing && !status?.model.busy && !status?.runtime.busy ? 'Checking compatibility and loading Bonsai…' : `${status?.runtime.phase === 'ready' ? 'Model' : 'Runtime'}: ${progress?.phase || 'preparing'}…`}</p>{progress?.busy && <progress aria-label="Bonsai installation progress" max={progress.total || 1} value={progress.received} />}</div>}
      <div className="connector-actions"><button type="button" className="primary" disabled={busy || active || otherInstall || !runtimeId || !info || insufficient} onClick={() => void install()}>Install and use Bonsai</button>
        {active && <button type="button" className="secondary" disabled={cancelling} onClick={async () => {
          setCancelling(true);
          try { await api.cancelBonsaiInstall(); }
          catch (e) { setError(errorMessage(e)); }
          finally { setCancelling(false); }
        }}>Cancel Bonsai installation</button>}
      </div>
    </>}
    {error && <><p className="error" role="alert">{error}</p>{nativeAvailable && <button type="button" className="secondary" disabled={busy || active || otherInstall || !runtimeId || !info || insufficient} onClick={() => void install('off')}>Install and run without MTP</button>}</>}
    {notice && <p className="success" role="status">{notice}</p>}
  </section>;
}
