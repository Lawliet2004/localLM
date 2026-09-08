import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig } from '../lib/types';

export function Execution() {
  const [config, setConfig] = useState<ExecutionConfig>({ pythonPath: '', nodePath: '', powershellPath: '' });
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [pendingCloud, setPendingCloud] = useState<Awaited<ReturnType<typeof api.pendingDaytonaOperations>>>([]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.pendingDaytonaOperations().then(value => { if (!disposed) setPendingCloud(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.getExecutionConfig().then(value => { if (!disposed) setConfig(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, []);
  async function browse(key: keyof ExecutionConfig) {
    try { const path = await open({ multiple: false, filters: [{ name: 'Executable', extensions: ['exe'] }] }); if (typeof path === 'string') setConfig(current => ({ ...current, [key]: path })); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function save() {
    setBusy(true); setError(''); setNotice('');
    try { await api.saveExecutionConfig(config); setNotice('Interpreter settings saved.'); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }
  return <div className="settings-page"><div className="page-heading"><p className="eyebrow">CODE & AUTOMATION</p><h1>Execution</h1><p>Run code with your installed Python, Node.js, or PowerShell interpreter.</p></div>
    <p className="catalog-notice">Local execution runs with your Windows account’s permissions, including network and filesystem access. It is not a sandbox. Ask and Auto-approve reads require approval of the complete code. Full access runs enabled code tools without prompts.</p>
    <form className="runtime-form" onSubmit={event => { event.preventDefault(); void save(); }}>
      {([['pythonPath', 'Python executable'], ['nodePath', 'Node.js executable'], ['powershellPath', 'PowerShell executable']] as const).map(([key, label]) => <label key={key}>{label}<div className="execution-path"><input aria-label={label} value={config[key]} placeholder="Not configured" disabled={busy} onChange={event => { setNotice(''); setConfig(current => ({ ...current, [key]: event.target.value })); }} /><button type="button" className="secondary" disabled={!nativeAvailable || busy} onClick={() => void browse(key)}>Browse</button></div></label>)}
      <button className="primary" disabled={!nativeAvailable || busy}>{busy ? 'Saving…' : 'Save interpreters'}</button>
    </form>
    {error && <p role="alert" className="error-banner">{error}</p>}{notice && <p role="status">{notice}</p>}
    <p className="catalog-notice">To use local execution, choose a workspace folder under Tools in chat and enable Local code. Runs have a 90-second maximum and capture up to 64 KiB from each output stream.</p>
    <p className="catalog-notice">Daytona cloud execution is still being integrated.</p>
    {pendingCloud.length > 0 && <section aria-label="Pending cloud cleanup"><h2>Cloud cleanup needs attention</h2><p>These operations may still have cloud resources. Their records are retained until removal is verified.</p>{pendingCloud.map(item => <div className="catalog-notice" key={item.name}><strong>{item.name}</strong><p>{item.sandboxId ? `Sandbox: ${item.sandboxId}` : 'Creation outcome unknown; look up this operation name before retrying.'}</p>{item.cleanupError && <p>{item.cleanupError}</p>}</div>)}</section>}
  </div>;
}
