import { useEffect, useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable, type ModelInstallStatus } from '../lib/api';
import { gib } from '../lib/runtimeGuidance';

// Lives outside the settings pages so navigating back to chat keeps progress visible.
export function DownloadActivity() {
  const [status, setStatus] = useState<ModelInstallStatus | null>(null);
  const [rate, setRate] = useState(0);
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState('');
  const [cancelling, setCancelling] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const sample = useRef({ received: 0, time: 0, changed: 0, path: '' });
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const next = await api.modelInstallStatus();
        if (disposed) return;
        const now = Date.now();
        const previous = sample.current;
        const same = previous.time > 0 && previous.path === next.path && next.received >= previous.received;
        const delta = same ? next.received - previous.received : 0;
        const changed = !same || delta > 0 ? now : previous.changed;
        setRate(same ? delta / Math.max((now - previous.time) / 1000, 0.001) : 0);
        setWaiting(next.busy && now - changed >= 10000);
        sample.current = { received: next.received, time: now, changed, path: next.path ?? '' };
        setStatus(next); setError('');
        if (next.busy) setDismissed(false);
      } catch (e) { if (!disposed) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void poll(), 1000); }
    }
    void poll();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  if (!status || dismissed || (!status.busy && !status.error && status.phase !== 'ready')) return null;
  const percent = status.total > 0 ? Math.min(100, status.received / status.total * 100) : null;
  const label = !status.busy ? status.phase === 'ready' ? 'Model download complete' : status.phase === 'cancelled' ? 'Download paused' : 'Download interrupted'
    : status.phase === 'verifying' ? 'Verifying model' : status.phase === 'resolving' || status.phase === 'preparing' ? 'Preparing model download' : 'Downloading model';
  return <section className="download-activity" aria-label="Model download">
    <div className="download-activity-copy"><strong role="status">{label}{percent !== null && status.busy ? ` · ${percent.toFixed(1)}%` : ''}</strong>
      {status.path && <small title={status.path}>{status.path.split(/[\\/]/).pop()}</small>}
      <span>{gib(status.received)} / {status.total ? gib(status.total) : 'checking size'}
        {status.busy && rate > 0 && status.phase !== 'verifying' ? ` · ${(rate / 1048576).toFixed(1)} MiB/s · ~${Math.max(1, Math.ceil((status.total - status.received) / rate / 60))} min left` : ''}
        {status.busy ? waiting ? ' · Waiting for progress…' : ' · You can keep chatting with a loaded model.' : ''}</span>
      {status.busy && <progress aria-label="Model download progress" max={status.total || 1} value={status.total ? status.received : undefined} />}
      {(error || status.error) && <span role="alert">{error || status.error} Saved partial files can be resumed in My Models.</span>}
    </div>
    {status.busy ? <button type="button" className="secondary" disabled={cancelling} onClick={async () => {
      setCancelling(true);
      try { await api.cancelModelInstall(); } catch (e) { setError(errorMessage(e)); }
      finally { setCancelling(false); }
    }}>{cancelling ? 'Pausing…' : 'Pause download'}</button> : <button type="button" className="secondary" onClick={() => setDismissed(true)}>Dismiss</button>}
  </section>;
}
