import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../../lib/api';
import type { RuntimeConfig } from '../../lib/types';
import { SettingsGroup, SettingRow } from '../../components/Settings';
import { ModelDownload, type ModelDownloadBackend } from '../../components/ModelDownload';
import { RuntimeDownload, type RuntimeDownloadBackend } from '../../components/RuntimeDownload';
import type { BonsaiInfo } from './types';

const filename = 'Bonsai-27B-Q1_0.gguf';
async function info() {
  const value = await api.bonsaiInfo();
  if (!value) throw new Error('Enable Experimental models before installing Bonsai.');
  return value;
}
const modelBackend: ModelDownloadBackend = {
  info: async () => (await info()).model,
  status: () => api.bonsaiInstallStatus('model'),
  install: () => api.installBonsai('model'),
  cancel: () => api.cancelBonsaiInstall('model'),
};
const runtimeBackend: RuntimeDownloadBackend = {
  info: async () => (await info()).runtime,
  status: () => api.bonsaiInstallStatus('runtime'),
  install: () => api.installBonsai('runtime'),
  cancel: () => api.cancelBonsaiInstall('runtime'),
  list: async () => [],
};

export function BonsaiSettings({ busy, modelPath, onActivated, onPreset, onRefresh }: {
  busy: boolean; modelPath: string; onActivated: () => void | Promise<void>;
  onPreset: (preset: RuntimeConfig) => void | Promise<void>;
  onRefresh?: () => void | Promise<void>;
}) {
  const [enabled, setEnabled] = useState(false);
  const [ready, setReady] = useState(!nativeAvailable);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [details, setDetails] = useState<BonsaiInfo | null>(null);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    void api.bonsaiEnabled().then(value => { if (!disposed) setEnabled(value); })
      .catch(e => { if (!disposed) setError(errorMessage(e)); })
      .finally(() => { if (!disposed) setReady(true); });
    return () => { disposed = true; };
  }, []);
  useEffect(() => {
    if (!enabled || !nativeAvailable) { setDetails(null); return; }
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try { const value = await api.bonsaiInfo(); if (!disposed) setDetails(value); }
      catch (e) { if (!disposed) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 1000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [enabled]);
  const selected = modelPath.replace(/\\/g, '/').endsWith(`/experimental/bonsai-27b/models/${filename}`);
  async function activate(usePreset?: boolean) {
    setPending(true); setError('');
    try { await api.activateBonsai(usePreset); await onActivated(); }
    catch (e) { setError(errorMessage(e)); try { await onRefresh?.(); } catch { /* Keep the activation error visible. */ } }
    finally { setPending(false); }
  }
  return <>
    <SettingsGroup>
      <SettingRow label="Experimental models" htmlFor="experimental-models" description="Opt in to additional models with separate downloads and runtimes.">
        <input id="experimental-models" className="switch" type="checkbox" checked={enabled}
          disabled={!nativeAvailable || !ready || busy || pending} onChange={async event => {
            const next = event.target.checked;
            setPending(true); setError('');
            try { setEnabled(await api.setBonsaiEnabled(next)); if (!next) await onRefresh?.(); }
            catch (e) { setError(errorMessage(e)); }
            finally { setPending(false); }
          }} />
      </SettingRow>
    </SettingsGroup>
    {error && <p className="error" role="alert">{error}</p>}
    {enabled && <section aria-label="Bonsai experimental model">
      <h2>Bonsai-27B · Q1_0 <span className="library-chip warning">Experimental</span></h2>
      <p className="selection-warning">Limited VRAM can slow responses on 4 GB GPUs. Expect 1-bit quality tradeoffs; multilingual quality may be weaker. Uses a separate PrismML CUDA runtime. Text only.</p>
      {pending && <p role="status">Applying experimental model settings…</p>}
      {details?.preset && <button type="button" className="secondary" disabled={busy || pending || !details.model.destinationExists || !details.runtimePath} onClick={async () => {
        if (!selected) { await activate(true); return; }
        setPending(true); setError('');
        try { await onPreset(details.preset!); } catch (e) { setError(errorMessage(e)); } finally { setPending(false); }
      }}>Apply measured Bonsai preset</button>}
      <ModelDownload filename={filename} label="Bonsai text weights" backend={modelBackend}
        verifiedMessage="Model verified. Use it below to load Bonsai with its separate runtime."
        busy={busy || pending || !details?.runtimePath} onSelect={() => void activate()} />
      <RuntimeDownload backend={runtimeBackend} title="PrismML · CUDA 12.4" selectable={false}
        description="Install the pinned PrismML runtime in a separate directory. Verify the text weights, then use the verified model."
        busy={busy || pending} onSelect={() => {}} />
      {details && !details.runtimePath && <p role="status">Install the PrismML runtime before using the model.</p>}
    </section>}
  </>;
}
