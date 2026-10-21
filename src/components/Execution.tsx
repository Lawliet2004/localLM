import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig, WebSearchConfig } from '../lib/types';

export function Execution({ section = 'execution' }: { section?: 'execution' | 'web-search' }) {
  const [config, setConfig] = useState<ExecutionConfig>({ pythonPath: '', nodePath: '', powershellPath: '' });
  const [busy, setBusy] = useState(nativeAvailable);
  const [executionLoaded, setExecutionLoaded] = useState(false);
  const [searchLoaded, setSearchLoaded] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [testResults, setTestResults] = useState<Record<string, { status: 'testing' | 'success' | 'error'; message: string }>>({});
  const [cloudKey, setCloudKey] = useState('');
  const [hasCloudKey, setHasCloudKey] = useState(false);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudRevision, setCloudRevision] = useState(0);
  const [cloudError, setCloudError] = useState('');
  const [pendingCloud, setPendingCloud] = useState<Awaited<ReturnType<typeof api.pendingDaytonaOperations>>>([]);
  const [webSearch, setWebSearch] = useState<WebSearchConfig>({ provider: 'searxng', searxngBaseUrl: 'http://127.0.0.1:8080', googleApiKey: '', hasGoogleApiKey: false, googleCxId: '', searchFallbackEnabled: true });
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchNotice, setSearchNotice] = useState('');
  const [searchError, setSearchError] = useState('');
  const [searchHealth, setSearchHealth] = useState('');

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getWebSearchConfig().then(value => { if (!disposed) { setWebSearch(current => ({ ...current, ...value, searchFallbackEnabled: Boolean(value.searchFallbackEnabled ?? current.searchFallbackEnabled) })); setSearchLoaded(true); } }).catch(e => { if (!disposed) setSearchError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.hasDaytonaKey().then(value => { if (!disposed) setHasCloudKey(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.getExecutionConfig().then(value => { if (!disposed) { setConfig(value); setExecutionLoaded(true); } }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
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

  async function browse(key: 'pythonPath' | 'nodePath' | 'powershellPath') {
    try {
      const path = await open({ multiple: false });
      if (typeof path === 'string') {
        setConfig(current => ({ ...current, [key]: path }));
        setTestResults(current => ({ ...current, [key]: { status: 'testing', message: '' } }));
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function testInterpreter(key: 'pythonPath' | 'nodePath' | 'powershellPath') {
    const path = config[key];
    if (!path.trim()) return;
    setTestResults(current => ({ ...current, [key]: { status: 'testing', message: 'Testing interpreter…' } }));
    try {
      const res = await api.testInterpreter(path);
      setTestResults(current => ({ ...current, [key]: { status: 'success', message: res } }));
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
          ...current,
          pythonPath: detected.pythonPath || current.pythonPath,
          nodePath: detected.nodePath || current.nodePath,
          powershellPath: detected.powershellPath || current.powershellPath,
          bashPath: detected.bashPath || current.bashPath,
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
    if (!executionLoaded) return;
    setBusy(true); setError(''); setNotice('');
    try { await api.saveExecutionConfig(config); setNotice('Interpreter settings saved.'); }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  async function saveSearch() {
    if (!searchLoaded) return;
    setSearchBusy(true); setSearchError(''); setSearchNotice('');
    try { await api.saveWebSearchConfig(webSearch); setSearchNotice('Search settings saved.'); }
    catch (e) { setSearchError(errorMessage(e)); }
    finally { setSearchBusy(false); }
  }

  async function checkSearch() {
    setSearchBusy(true); setSearchHealth('');
    try {
      const res = await api.webSearchHealth(webSearch);
      const diagnostics = res.searxngDiagnostics as { engines?: Record<string, string> } | undefined;
      const failures = Object.entries(diagnostics?.engines || {}).map(([engine, reason]) => `${engine}: ${reason}`).join('; ');
      setSearchHealth([
        `Search: ${String(res.search ?? 'unknown')}`,
        typeof res.searchError === 'string' ? res.searchError : '',
        failures ? `Engine issues: ${failures}` : '',
      ].filter(Boolean).join('. '));
    } catch (e) { setSearchHealth(`Check failed: ${errorMessage(e)}`); }
    finally { setSearchBusy(false); }
  }

  return (
    <div className="settings-page">
      <div className="page-heading">
        <h1>{section === 'web-search' ? 'Web Search' : 'Execution'}</h1>
        <p>{section === 'web-search' ? 'Configure retrieval, research depth and persistent caches.' : 'Run code with your installed Python, Node.js, or PowerShell interpreter.'}</p>
      </div>
      {section === 'execution' && <>
      <section className="preference-section" aria-labelledby="interpreters-heading">
        <div className="preference-section-heading"><h2 id="interpreters-heading">Interpreters</h2><p>Local execution runs with your account’s permissions, including network and filesystem access. It is not a sandbox.</p></div>
        <form onSubmit={event => { event.preventDefault(); void save(); }}>
          <div className="preference-group">
            {([['pythonPath', 'Python executable'], ['nodePath', 'Node.js executable'], ['powershellPath', 'PowerShell executable']] as const).map(([key, label]) => (
              <div className="preference-row preference-row-block" key={key}>
                <div className="preference-copy">
                  <label htmlFor={`exec-${key}`}>{label}</label>
                </div>
                <div className="field-row">
                  <input
                    id={`exec-${key}`}
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
              </div>
            ))}
            <div className="connector-actions">
              <button className="primary" disabled={!nativeAvailable || busy || !executionLoaded}>
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
          </div>
        </form>
        <p className="preference-footnote">Ask and Auto-approve reads require approval of the complete code. Full access follows the separate Bash and Python approval settings.</p>
      </section>
      {error && <p role="alert" className="error-banner">{error}</p>}
      {cloudError && <p role="alert" className="error-banner">Cloud cleanup status could not be refreshed: {cloudError}</p>}
      {notice && <p role="status">{notice}</p>}</>}
      <section className="preference-section" aria-labelledby="web-search-heading">
        <div className="preference-section-heading"><h2 id="web-search-heading">Web search</h2><p>Connect a self-hosted SearXNG instance, or use Google Custom Search.</p></div>
        <form onSubmit={event => { event.preventDefault(); void saveSearch(); }}>
          <div className="preference-group">
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <label htmlFor="web-search-enabled">Web research</label>
                <p>Allow research, page retrieval, and weather lookups.</p>
              </div>
              <input id="web-search-enabled" className="switch" type="checkbox" aria-label="Enable web research" checked={webSearch.enabled ?? true} disabled={!nativeAvailable || searchBusy} onChange={event => { setSearchNotice(''); setWebSearch(current => ({ ...current, enabled: event.target.checked })); }} />
            </div>
            <div className="preference-row">
              <div className="preference-copy">
                <label htmlFor="web-search-depth">Research depth</label>
                <p>Query, page, and deadline budgets per request. Models can request a depth explicitly.</p>
              </div>
              <select id="web-search-depth" value={webSearch.mode || 'normal'} disabled={!nativeAvailable || searchBusy} onChange={event => { setSearchNotice(''); setWebSearch(current => ({ ...current, mode: event.target.value as WebSearchConfig['mode'] })); }}><option value="fast">Fast</option><option value="normal">Balanced</option><option value="deep">Deep</option></select>
            </div>
            <div className="preference-row">
              <div className="preference-copy">
                <label htmlFor="web-search-provider">Provider</label>
                <p>Where search requests are sent.</p>
              </div>
              <select
                id="web-search-provider"
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
            </div>
            {webSearch.provider === 'searxng' ? (
              <div className="preference-row preference-row-block">
                <div className="preference-copy">
                  <label htmlFor="searxng-url">SearXNG base URL</label>
                  <p>Instance URL including any path prefix; separate multiple instances with commas. Enable json under search.formats in settings.yml.</p>
                </div>
                <input
                  id="searxng-url"
                  aria-label="SearXNG base URL"
                  value={webSearch.searxngBaseUrl}
                  placeholder="http://127.0.0.1:8080"
                  disabled={!nativeAvailable || searchBusy}
                  onChange={event => {
                    setSearchNotice('');
                    setWebSearch(current => ({ ...current, searxngBaseUrl: event.target.value }));
                  }}
                />
              </div>
            ) : (
              <>
                <div className="preference-row preference-row-block">
                  <div className="preference-copy">
                    <label htmlFor="google-api-key">Google API key</label>
                    <p>The key is stored in the protected credential vault, never in plain text.</p>
                  </div>
                  <input
                    id="google-api-key"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    aria-label="Google API key"
                    placeholder={webSearch.hasGoogleApiKey ? 'A key is stored — type to replace it' : 'Paste the Google API key'}
                    value={webSearch.googleApiKey ?? ''}
                    disabled={!nativeAvailable || searchBusy}
                    onChange={event => {
                      setSearchNotice('');
                      setWebSearch(current => ({ ...current, googleApiKey: event.target.value, clearGoogleApiKey: false }));
                    }}
                  />
                </div>
                {webSearch.hasGoogleApiKey && (
                  <div className="preference-row preference-toggle">
                    <div className="preference-copy">
                      <label htmlFor="google-api-key-clear">Clear the stored key</label>
                      <p>The stored key is removed when the settings are saved.</p>
                    </div>
                    <input
                      id="google-api-key-clear"
                      className="switch"
                      type="checkbox"
                      aria-label="Clear the stored Google API key"
                      checked={Boolean(webSearch.clearGoogleApiKey)}
                      disabled={!nativeAvailable || searchBusy}
                      onChange={event => {
                        setSearchNotice('');
                        setWebSearch(current => ({ ...current, clearGoogleApiKey: event.target.checked }));
                      }}
                    />
                  </div>
                )}
                <div className="preference-row preference-row-block">
                  <div className="preference-copy">
                    <label htmlFor="google-cx-id">Google Custom Search Engine ID</label>
                  </div>
                  <input
                    id="google-cx-id"
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
                </div>
              </>
            )}
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <label htmlFor="web-search-fallback">Provider fallback</label>
                <p>Try the other provider when the primary returns no results. Google fallback is capped at 90 calls/day.</p>
              </div>
              <input
                id="web-search-fallback"
                className="switch"
                type="checkbox"
                aria-label="Fall back to the other search provider on empty results"
                checked={Boolean(webSearch.searchFallbackEnabled)}
                disabled={!nativeAvailable || searchBusy}
                onChange={event => {
                  setSearchNotice('');
                  setWebSearch(current => ({ ...current, searchFallbackEnabled: event.target.checked }));
                }}
              />
            </div>
            <div className="connector-actions">
              <button className="primary" disabled={!nativeAvailable || searchBusy || !searchLoaded}>
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
              <button type="button" className="secondary" disabled={!nativeAvailable || searchBusy} onClick={() => {
                setSearchBusy(true); setSearchError(''); setSearchNotice('');
                void api.clearWebCache().then(result => { if (result.cleared) setSearchNotice('Research and weather caches cleared. Saved research sessions and citations are retained.'); }).catch(e => setSearchError(errorMessage(e))).finally(() => setSearchBusy(false));
              }}>Clear retrieval cache</button>
            </div>
          </div>
        </form>
        {searchHealth && <p role="status">{searchHealth}</p>}
        {searchError && <p role="alert" className="error-banner">{searchError}</p>}
        {searchNotice && <p role="status">{searchNotice}</p>}
      </section>
      {section === 'execution' && <>
      <section className="preference-section" aria-labelledby="cloud-heading">
        <div className="preference-section-heading"><h2 id="cloud-heading">Daytona credentials</h2><p>{hasCloudKey ? 'An encrypted API key is saved.' : 'No Daytona key saved.'} Cloud execution cannot be started; a saved key stays until you choose Forget.</p></div>
        <form onSubmit={event => { event.preventDefault(); void cloudAction('save'); }}>
          <div className="preference-group">
            <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor="daytona-key">Daytona API key</label>
                <p>Saving a key does not create a sandbox or verify account access.</p>
              </div>
              <input
                id="daytona-key"
                type="password"
                autoComplete="off"
                spellCheck={false}
                aria-label="Daytona API key"
                value={cloudKey}
                disabled={!nativeAvailable || cloudBusy}
                onChange={event => setCloudKey(event.target.value)}
              />
            </div>
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
          </div>
        </form>
        <p className="preference-footnote">To use local execution, choose a workspace folder under Tools and enable Local code. Runs have a 90-second maximum and capture up to 64 KiB per output stream.</p>
      </section>
      {pendingCloud.length > 0 && (
        <section className="preference-section" aria-label="Pending cloud cleanup">
          <div className="preference-section-heading"><h2>Cloud cleanup needs attention</h2><p>These operations may still have cloud resources. Their records are retained until removal is verified.</p></div>
          <div className="preference-group">
            {pendingCloud.map(item => (
              <div className="settings-note" key={item.name}>
                <p><strong>{item.name}</strong></p>
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
          </div>
        </section>
      )}
      </>}
    </div>
  );
}
