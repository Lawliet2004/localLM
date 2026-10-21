import { LatencyDiagnostic } from './LatencyDiagnostic';
import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';

export function RuntimeDiagnostics() {
  const [revision, setRevision] = useState(0);
  const [log, setLog] = useState<{ content: string; truncated: boolean } | null>(null);
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [probing, setProbing] = useState(false);
  const [probeResult, setProbeResult] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    setBusy(true); setError('');
    api.readRuntimeLog().then(value => { if (!disposed) setLog(value); })
      .catch(e => { if (!disposed) setError(errorMessage(e)); })
      .finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, [revision]);
  return <section aria-label="Runtime diagnostics">
    <LatencyDiagnostic />
    <div className="form-footer">
      <h2>Runtime log</h2>
      <div style={{ display: 'flex', gap: 8 }}>
        <button type="button" className="secondary" disabled={!nativeAvailable || probing} onClick={() => {
          setProbing(true); setError(''); setProbeResult('');
          void api.retestToolCalling().then(value => setProbeResult(value.message)).catch(e => setError(errorMessage(e))).finally(() => setProbing(false));
        }}>{probing ? 'Testing tool calling…' : 'Retest tool calling'}</button>
        {probing && <button type="button" className="secondary" onClick={() => void api.cancelGeneration().catch(e => setError(errorMessage(e)))}>Cancel test</button>}
        <button className="secondary" disabled={!nativeAvailable || busy} onClick={() => setRevision(value => value + 1)}>{busy ? 'Reading…' : 'Refresh log'}</button>
      </div>
    </div>
    <p>Output from the most recent model load. Logs reset when you load a model and stop recording at 8 MiB.</p>
    <p>The tool test sends a small request to the loaded local model and checks its structured response. It executes no tool. The test can replace a warm prompt cache in a single-slot runtime.</p>
    {probeResult && <p>{probeResult}</p>}
    {error && <p role="alert" className="error-banner">{error}</p>}
    {log?.truncated && <p role="status">Showing the final 64 KiB of the saved log; the first line may be incomplete.</p>}
    {log?.content ? <pre className="runtime-diagnostics-log" tabIndex={0} aria-label="Runtime log output">{log.content}</pre> : !busy && !error && <p>{nativeAvailable ? 'No runtime output recorded yet.' : 'Runtime diagnostics are available in the desktop app.'}</p>}
  </section>;
}
