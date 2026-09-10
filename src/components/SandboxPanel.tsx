import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';

export function SandboxPanel() {
  const [status, setStatus] = useState({ provider: 'local', warning: '', docker: 'unknown' });
  const [image, setImage] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(nativeAvailable);
  async function refresh() {
    if (!nativeAvailable) return;
    try { setStatus(await api.sandboxStatus()); }
    catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  useEffect(() => { void refresh(); }, []);
  async function change(provider: string) {
    setError('');
    try { await api.setSandboxProvider(provider, image.trim() || undefined); await refresh(); }
    catch (e) { setError(errorMessage(e)); }
  }
  return (
    <section aria-label="Sandbox provider">
      <h2>Sandbox</h2>
      <p>{status.warning || 'Local execution is NOT an isolation boundary. Use Docker for untrusted work.'}</p>
      {error && <p role="alert" className="error-banner">{error}</p>}
      <p>Provider: <strong>{status.provider}</strong> · Docker: {status.docker}</p>
      <label>Docker image <input aria-label="Docker image" value={image} onChange={e => setImage(e.target.value)} placeholder="mcr.microsoft.com/windows/servercore:ltsc2022" /></label>
      <div className="row-actions">
        <button type="button" className="secondary" disabled={busy} onClick={() => void change('local')}>Use local</button>
        <button type="button" className="secondary" disabled={busy} onClick={() => void change('docker')}>Use Docker</button>
      </div>
    </section>
  );
}
