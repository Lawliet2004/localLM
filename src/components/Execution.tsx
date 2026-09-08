import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig } from '../lib/types';

export function Execution() {
  const [config, setConfig] = useState<ExecutionConfig>({ pythonPath: '', nodePath: '', powershellPath: '' });
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [cloudKey, setCloudKey] = useState('');
  const [hasCloudKey, setHasCloudKey] = useState(false);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudRevision, setCloudRevision] = useState(0);
  const [cloudError, setCloudError] = useState('');
  const [pendingCloud, setPendingCloud] = useState<Awaited<ReturnType<typeof api.pendingDaytonaOperations>>>([]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.hasDaytonaKey().then(value => { if (!disposed) setHasCloudKey(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.getExecutionConfig().then(value => { if (!disposed) setConfig(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, []);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function refresh() {
      try {
        const value = await api.pendingDaytonaOperations();
        if (!disposed) { setPendingCloud(value); setCloudError(''); }
      } catch (e) {
        if (!disposed) setCloudError(errorMessage(e));
      } finally {
        if (!disposed) timer = setTimeout(() => void refresh(), 5000);
      }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [cloudRevision]);
  async function browse(key: keyof ExecutionConfig) {
    try { const path = await open({ multiple: false, filters: [{ name: 'Executable', extensions: ['exe'] }] }); if (typeof path === 'string') setConfig(current => ({ ...current, [key]: path })); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function cloudAction(action: 'save' | 'forget' | 'cleanup', name?: string) {
    setCloudBusy(true); setError(''); setNotice('');
    try {
      if (action === 'save') { await api.saveDaytonaKey(cloudKey); setCloudKey(''); setHasCloudKey(true); setNotice('Daytona key saved securely. Account access has not been verified.'); }
      if (action === 'forget') { await api.forgetDaytonaKey(); setCloudKey(''); setHasCloudKey(false); setNotice('Daytona key removed.'); }
      if (action === 'cleanup' && name) { await api.retryDaytonaCleanup(name); setNotice('Cloud resource removal confirmed.'); }
    } catch (e) { setError(errorMessage(e)); }
    finally { setCloudRevision(value => value + 1); setCloudBusy(false); }
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
    {error && <p role="alert" className="error-banner">{error}</p>}{cloudError && <p role="alert" className="error-banner">Cloud cleanup status could not be refreshed: {cloudError}</p>}{notice && <p role="status">{notice}</p>}
    <p className="catalog-notice">To use local execution, choose a workspace folder under Tools in chat and enable Local code. Runs have a 90-second maximum and capture up to 64 KiB from each output stream.</p>
    <p className="catalog-notice">Enable Daytona cloud code under Tools in chat to run Python, JavaScript or TypeScript in a temporary remote sandbox. Code leaves this device and cloud usage may incur charges. Local workspace files are not uploaded automatically. Cleanup is attempted after every run; unresolved resources appear below.</p>
    <form className="runtime-form" onSubmit={event => { event.preventDefault(); void cloudAction('save'); }}><h2>Daytona credentials</h2><p>{hasCloudKey ? 'An encrypted API key is saved.' : 'No Daytona key saved.'} Saving a key does not create a sandbox or verify account access.</p><label>Daytona API key<input type="password" autoComplete="off" spellCheck={false} value={cloudKey} disabled={!nativeAvailable || cloudBusy} onChange={event => setCloudKey(event.target.value)} /></label><div className="connector-actions"><button className="primary" disabled={!nativeAvailable || cloudBusy || !cloudKey}>Save Daytona key</button>{hasCloudKey && <button type="button" className="secondary" disabled={cloudBusy || pendingCloud.length > 0} onClick={() => void cloudAction('forget')}>Forget Daytona key</button>}</div></form>
    {pendingCloud.length > 0 && <section aria-label="Pending cloud cleanup"><h2>Cloud cleanup needs attention</h2><p>These operations may still have cloud resources. Their records are retained until removal is verified.</p>{pendingCloud.map(item => <div className="catalog-notice" key={item.name}><strong>{item.name}</strong><p>{item.sandboxId ? `Sandbox: ${item.sandboxId}` : 'Creation outcome unknown; look up this operation name before retrying.'}</p>{item.cleanupError && <p>{item.cleanupError}</p>}<button className="secondary" disabled={!hasCloudKey || cloudBusy} onClick={() => void cloudAction('cleanup', item.name)}>Retry cleanup</button></div>)}</section>}
  </div>;
}
