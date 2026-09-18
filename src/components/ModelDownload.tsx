import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelDownloadInfo, type ModelInstallStatus } from '../lib/api';
import { bonsaiFilename, localModels, zaya1Filename } from '../lib/localModels';
const gib = (bytes: number) => `${(bytes / 1073741824).toFixed(2)} GiB`;
export function ModelDownload({ busy, onSelect, filename = localModels[0].filename }: { busy: boolean; onSelect: (path: string) => void; filename?: string }) {
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
    setInfo(null); setStatus(null); setError('');
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      const version = actionVersion.current;
      try {
        const [nextInfo, nextStatus] = await Promise.all([api.modelDownloadInfo(filename), api.modelInstallStatus()]);
        if (!disposed && version === actionVersion.current) { setInfo(nextInfo); setStatus(nextStatus); }
      } catch (e) { if (!disposed && version === actionVersion.current) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 1000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [revision, filename]);
  async function install() {
    actionVersion.current += 1;
    setStarting(true); setError('');
    try { setStatus(await api.installModel(filename)); }
    catch (e) { setError(errorMessage(e)); }
    finally { actionVersion.current += 1; setStarting(false); setRevision(value => value + 1); }
  }
  const active = starting || status?.busy;
  const verified = status?.phase === 'ready' && status.path === info?.destination;
  const insufficient = info && !info.destinationExists && info.availableBytes < info.requiredBytes;
  return <section className="catalog-notice" aria-label="Recommended model download">
    <h2>{localModels.find(model => model.filename === filename)?.label ?? filename}</h2>
    {filename === bonsaiFilename && <p>Requires a Prism llama.cpp build for legacy group-128 Q2_0: prism-b9601-68faa14. <a href="https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b9601-68faa14" target="_blank" rel="noreferrer">Get the compatible runtime</a>. Extract its Windows CUDA 12.4 runtime and CUDA DLL archives into the same folder, then select llama-server.exe below. Supports the model's full 65,536-token context; lower it in Runtime if your available memory cannot accommodate the larger KV cache.</p>}
    {filename === zaya1Filename && <p>This model uses the experimental ZAYA architecture from <a href="https://github.com/ggml-org/llama.cpp/pull/23112" target="_blank" rel="noreferrer">llama.cpp PR #23112</a>. When you load it, the app will automatically configure the custom runtime and optimal CPU settings. If the custom runtime is not yet built, run <code>scripts/prepare-zaya-runtime.ps1</code> first. This community GGUF is 5.19 GiB and uses system RAM. Local ZAYA turns currently answer without tool calling because this experimental runtime does not reliably parse structured tool calls. See docs/ZAYA1-RUNTIME.md for details.</p>}
    <p>Download the pinned GGUF from Hugging Face, or verify an existing managed copy. The file is checked against its SHA-256 before use.</p>
    {info && <><p>{gib(info.bytes)} download · {gib(info.availableBytes)} free · {gib(info.requiredBytes)} required including reserve</p><details><summary>File details</summary><p className="download-path">{info.destination}</p><code className="download-path">SHA-256: {info.sha256}</code></details></>}
    {insufficient && <p role="alert">Not enough disk space for this download.</p>}
    {error && <p role="alert">{error}</p>}{status?.error && <p role={status.phase === 'cancelled' ? 'status' : 'alert'}>{status.phase === 'cancelled' ? 'Installation cancelled. You can try again when ready.' : status.error}</p>}
    {active && <div role="status"><p>{status?.phase === 'verifying' ? 'Verifying existing model' : 'Downloading model'} · {gib(status?.received ?? 0)} / {gib(status?.total || info?.bytes || 0)}</p><progress aria-label="Model installation progress" max={status?.total || info?.bytes || 1} value={status?.received ?? 0} /></div>}
    {verified && <p role="status">Model verified. Select it below, then save your model settings.</p>}
    <div className="connector-actions"><button type="button" className="secondary" disabled={!nativeAvailable || !info || active || Boolean(insufficient)} onClick={() => void install()}>{info?.destinationExists ? 'Verify managed model' : 'Download model'}</button>
      {active && <button type="button" className="secondary" disabled={cancelling} onClick={async () => { setCancelling(true); try { await api.cancelModelInstall(); } catch (e) { setError(errorMessage(e)); } finally { setCancelling(false); } }}>Cancel installation</button>}
      {verified && status?.path && <button type="button" className="primary" disabled={busy || Boolean(active)} onClick={() => onSelect(status.path!)}>Use verified model</button>}</div>
  </section>;
}
