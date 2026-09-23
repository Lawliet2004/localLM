import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';

export function RuntimeDiagnostics() {
  const [revision, setRevision] = useState(0);
  const [log, setLog] = useState<{ content: string; truncated: boolean } | null>(null);
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [toolRetest, setToolRetest] = useState('');
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
    <div className="form-footer">
      <h2>Runtime log</h2>
      <div style={{ display: 'flex', gap: 8 }}>
        <button type="button" className="secondary" onClick={() => setToolRetest('Tool calling will be offered again on the next message. A malformed call is repaired once without disabling later turns.')}>Retest tool calling</button>
        <button className="secondary" disabled={!nativeAvailable || busy} onClick={() => setRevision(value => value + 1)}>{busy ? 'Reading…' : 'Refresh log'}</button>
      </div>
    </div>
    <p>Output from the most recent model load. Logs reset when you load a model and stop recording at 8 MiB.</p>
    {toolRetest && <p>{toolRetest}</p>}
    {error && <p role="alert" className="error-banner">{error}</p>}
    {log?.truncated && <p role="status">Showing the final 64 KiB of the saved log; the first line may be incomplete.</p>}
    {log?.content ? <pre className="runtime-diagnostics-log" tabIndex={0} aria-label="Runtime log output">{log.content}</pre> : !busy && !error && <p>{nativeAvailable ? 'No runtime output recorded yet.' : 'Runtime diagnostics are available in the desktop app.'}</p>}
  </section>;
}
