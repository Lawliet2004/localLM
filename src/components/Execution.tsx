import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig, WebSearchConfig } from '../lib/types';

export function Execution() {
  const [config, setConfig] = useState<ExecutionConfig>({ pythonPath: '', nodePath: '', powershellPath: '' });
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [testResults, setTestResults] = useState<Record<string, { status: 'testing' | 'success' | 'error'; message: string }>>({});
  const [cloudKey, setCloudKey] = useState('');
  const [hasCloudKey, setHasCloudKey] = useState(false);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudRevision, setCloudRevision] = useState(0);
  const [cloudError, setCloudError] = useState('');
  const [pendingCloud, setPendingCloud] = useState<Awaited<ReturnType<typeof api.pendingDaytonaOperations>>>([]);
  const [webSearch, setWebSearch] = useState<WebSearchConfig>({ provider: 'searxng', searxngBaseUrl: 'http://127.0.0.1:8080', googleApiKey: '', googleCxId: '', searchFallbackEnabled: true });
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchNotice, setSearchNotice] = useState('');
  const [searchError, setSearchError] = useState('');
  const [searchHealth, setSearchHealth] = useState('');

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getWebSearchConfig().then(value => { if (!disposed) setWebSearch(current => ({ ...current, ...value, searchFallbackEnabled: Boolean(value.searchFallbackEnabled ?? current.searchFallbackEnabled) })); }).catch(e => { if (!disposed) setSearchError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);

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
    try {
      const path = await open({ multiple: false, filters: [{ name: 'Executable', extensions: ['exe'] }] });
      if (typeof path === 'string') {
        setConfig(current => ({ ...current, [key]: path }));
        setTestResults(current => ({ ...current, [key]: { status: 'testing', message: '' } }));
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function testInterpreter(key: keyof ExecutionConfig) {
    const path = config[key];
    if (!path.trim()) return;
    setTestResults(current => ({ ...current, [key]: { status: 'testing', message: 'Testing interpreter…' } }));
    try {
      if (api.testInterpreter) {
        const res = await api.testInterpreter(path);
        setTestResults(current => ({ ...current, [key]: { status: 'success', message: res } }));
      } else {
        setTestResults(current => ({ ...current, [key]: { status: 'success', message: 'Interpreter verified.' } }));
      }
    } catch (e) {
      setTestResults(current => ({ ...current, [key]: { status: 'error', message: errorMessage(e) } }));
    }
  }

  async function autoDetect() {
    setBusy(true); setError(''); setNotice('');
    try {
      if (api.detectInterpreters) {
        const detected = await api.detectInterpreters();
        setConfig(current => ({
          pythonPath: detected.pythonPath || current.pythonPath,
          nodePath: detected.nodePath || current.nodePath,
          powershellPath: detected.powershellPath || current.powershellPath,
        }));
        const found = [
          detected.pythonPath && 'Python',
          detected.nodePath && 'Node.js',
          detected.powershellPath && 'PowerShell',
        ].filter(Boolean);
        setNotice(
          found.length > 0
            ? `Detected from system PATH: ${found.join(', ')}. You do not need to type these paths.`
            : 'No interpreter was found on PATH. Install Node from the managed runtime path, or install Python, then detect again. Do not hand-edit a config file.'
        );
      } else {
        setNotice('Interpreter detection is unavailable in mock environment.');
      }
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
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

  async function saveSearch() {
    setSearchBusy(true); setSearchError(''); setSearchNotice('');
    try { await api.saveWebSearchConfig(webSearch); setSearchNotice('Search settings saved.'); }
    catch (e) { setSearchError(errorMessage(e)); }
    finally { setSearchBusy(false); }
  }

  async function checkSearch() {
    setSearchBusy(true); setSearchHealth('');
    try {
      const res = await api.webSearchHealth();
      setSearchHealth(`Search: ${String(res.search ?? 'unknown')}`);
    } catch (e) { setSearchHealth(`Check failed: ${errorMessage(e)}`); }
    finally { setSearchBusy(false); }
  }

  return (
    <div className="settings-page">
      <div className="page-heading">
        <p className="eyebrow">CODE & AUTOMATION</p>
        <h1>Execution</h1>
        <p>Run code with your installed Python, Node.js, or PowerShell interpreter.</p>
      </div>
      <p className="catalog-notice">
        Local execution runs with your Windows account’s permissions, including network and filesystem access. It is not a sandbox. Ask and Auto-approve reads require approval of the complete code. Full access runs enabled code tools without prompts.
      </p>
      <form className="runtime-form" onSubmit={event => { event.preventDefault(); void save(); }}>
        {([['pythonPath', 'Python executable'], ['nodePath', 'Node.js executable'], ['powershellPath', 'PowerShell executable']] as const).map(([key, label]) => (
          <label key={key}>
            {label}
            <div className="execution-path">
              <input
                aria-label={label}
                value={config[key]}
                placeholder="Not configured"
                disabled={busy}
                onChange={event => {
                  setNotice('');
                  setConfig(current => ({ ...current, [key]: event.target.value }));
                }}
              />
              <button
                type="button"
                className="secondary"
                disabled={!nativeAvailable || busy || !config[key].trim()}
                onClick={() => void testInterpreter(key)}
              >
                {testResults[key]?.status === 'testing' ? 'Testing…' : 'Test'}
              </button>
              <button
                type="button"
                className="secondary"
                disabled={!nativeAvailable || busy}
                onClick={() => void browse(key)}
              >
                Browse
              </button>
            </div>
            {testResults[key] && testResults[key].message && (
              <small className={`test-feedback ${testResults[key].status}`}>
                {testResults[key].message}
              </small>
            )}
          </label>
        ))}
        <div className="connector-actions">
          <button className="primary" disabled={!nativeAvailable || busy}>
            {busy ? 'Saving…' : 'Save interpreters'}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={!nativeAvailable || busy}
            onClick={() => void autoDetect()}
          >
            Auto-detect Interpreters
          </button>
        </div>
      </form>
      {error && <p role="alert" className="error-banner">{error}</p>}
      {cloudError && <p role="alert" className="error-banner">Cloud cleanup status could not be refreshed: {cloudError}</p>}
      {notice && <p role="status">{notice}</p>}
      <form className="runtime-form" onSubmit={event => { event.preventDefault(); void saveSearch(); }}>
        <h2>Web search</h2>
        <p>Choose the search provider for deep research. SearXNG is an explicit install of the pinned image searxng/searxng:2026.9.22-019460e07, not latest. Google uses the Custom Search JSON API (100 queries/day free tier).</p>
        <label>
          Search provider
          <select
            aria-label="Search provider"
            value={webSearch.provider}
            disabled={!nativeAvailable || searchBusy}
            onChange={event => {
              setSearchNotice('');
              setWebSearch(current => ({ ...current, provider: event.target.value as WebSearchConfig['provider'] }));
            }}
          >
            <option value="searxng">SearXNG (self-hosted)</option>
            <option value="google">Google Custom Search</option>
          </select>
        </label>
        {webSearch.provider === 'searxng' ? (
          <label>
            SearXNG base URL
            <input
              aria-label="SearXNG base URL"
              value={webSearch.searxngBaseUrl}
              placeholder="http://127.0.0.1:8080"
              disabled={!nativeAvailable || searchBusy}
              onChange={event => {
                setSearchNotice('');
                setWebSearch(current => ({ ...current, searxngBaseUrl: event.target.value }));
              }}
            />
          </label>
        ) : (
          <>
            <label>
              Google API key
              <input
                type="password"
                autoComplete="off"
                spellCheck={false}
                aria-label="Google API key"
                value={webSearch.googleApiKey}
                disabled={!nativeAvailable || searchBusy}
                onChange={event => {
                  setSearchNotice('');
                  setWebSearch(current => ({ ...current, googleApiKey: event.target.value }));
                }}
              />
            </label>
            <label>
              Google Custom Search Engine ID
              <input
                autoComplete="off"
                spellCheck={false}
                aria-label="Google Custom Search Engine ID"
                value={webSearch.googleCxId}
                disabled={!nativeAvailable || searchBusy}
                onChange={event => {
                  setSearchNotice('');
                  setWebSearch(current => ({ ...current, googleCxId: event.target.value }));
                }}
              />
            </label>
          </>
        )}
        <div className="connector-actions">
          <button className="primary" disabled={!nativeAvailable || searchBusy}>
            {searchBusy ? 'Saving…' : 'Save search settings'}
          </button>
          <button
            type="button"
            className="secondary"
            disabled={!nativeAvailable || searchBusy}
            onClick={() => void checkSearch()}
          >
            Check connection
          </button>
        </div>
        <label>
          <input
            type="checkbox"
            aria-label="Fall back to the other search provider on empty results"
            checked={Boolean(webSearch.searchFallbackEnabled)}
            disabled={!nativeAvailable || searchBusy}
            onChange={event => {
              setSearchNotice('');
              setWebSearch(current => ({ ...current, searchFallbackEnabled: event.target.checked }));
            }}
          />
          Fall back to the other provider when the primary returns no results (Google fallback capped at 90 calls/day)
        </label>
        {searchHealth && <p role="status">{searchHealth}</p>}
        {searchError && <p role="alert" className="error-banner">{searchError}</p>}
        {searchNotice && <p role="status">{searchNotice}</p>}
      </form>
      <p className="catalog-notice">
        To use local execution, choose a workspace folder under Tools in chat and enable Local code. Runs have a 90-second maximum and capture up to 64 KiB from each output stream.
      </p>
      <p className="catalog-notice">
        Cloud execution cannot be started. Unresolved historical cloud operations stay listed below and are not erased. A saved key stays until you choose Forget. Local code is unsandboxed.
      </p>
      <form className="runtime-form" onSubmit={event => { event.preventDefault(); void cloudAction('save'); }}>
        <h2>Daytona credentials</h2>
        <p>{hasCloudKey ? 'An encrypted API key is saved.' : 'No Daytona key saved.'} Saving a key does not create a sandbox or verify account access.</p>
        <label>
          Daytona API key
          <input
            type="password"
            autoComplete="off"
            spellCheck={false}
            value={cloudKey}
            disabled={!nativeAvailable || cloudBusy}
            onChange={event => setCloudKey(event.target.value)}
          />
        </label>
        <div className="connector-actions">
          <button className="primary" disabled={!nativeAvailable || cloudBusy || !cloudKey}>Save Daytona key</button>
          {hasCloudKey && (
            <button
              type="button"
              className="secondary"
              disabled={cloudBusy || pendingCloud.length > 0}
              onClick={() => void cloudAction('forget')}
            >
              Forget Daytona key
            </button>
          )}
        </div>
      </form>
      {pendingCloud.length > 0 && (
        <section aria-label="Pending cloud cleanup">
          <h2>Cloud cleanup needs attention</h2>
          <p>These operations may still have cloud resources. Their records are retained until removal is verified.</p>
          {pendingCloud.map(item => (
            <div className="catalog-notice" key={item.name}>
              <strong>{item.name}</strong>
              <p>{item.sandboxId ? `Sandbox: ${item.sandboxId}` : 'Creation outcome unknown; look up this operation name before retrying.'}</p>
              {item.cleanupError && <p>{item.cleanupError}</p>}
              <button
                className="secondary"
                disabled={!hasCloudKey || cloudBusy}
                onClick={() => void cloudAction('cleanup', item.name)}
              >
                Retry cleanup
              </button>
            </div>
          ))}
        </section>
      )}
    </div>
  );
}
