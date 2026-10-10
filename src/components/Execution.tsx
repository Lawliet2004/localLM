import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig, IsolationSettings, IsolationStatus, WebSearchConfig } from '../lib/types';
import { SettingRow, SettingsGroup, SettingsHeader } from './Settings';

export function Execution({ section = 'execution' }: { section?: 'execution' | 'web-search' }) {
  const [config, setConfig] = useState<ExecutionConfig>({ pythonPath: '', nodePath: '', powershellPath: '' });
  const latestConfig = useRef(config);
  latestConfig.current = config;
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
  const [webSearch, setWebSearch] = useState<WebSearchConfig>({ provider: 'searxng', searxngBaseUrl: 'http://127.0.0.1:8080', hasGoogleApiKey: false, googleCxId: '', searchFallbackEnabled: true });
  const [googleApiKey, setGoogleApiKey] = useState('');
  const [clearGoogleApiKey, setClearGoogleApiKey] = useState(false);
  const [searchBusy, setSearchBusy] = useState(false);
  const [searchNotice, setSearchNotice] = useState('');
  const [searchError, setSearchError] = useState('');
  const [searchHealth, setSearchHealth] = useState('');
  const [isolation, setIsolation] = useState<IsolationSettings | null>(null);
  const [isolationStatus, setIsolationStatus] = useState<IsolationStatus | null>(null);

  useEffect(() => {
    if (!nativeAvailable || section !== 'web-search') return;
    let disposed = false;
    api.getWebSearchConfig().then(value => { if (!disposed) setWebSearch(current => ({ ...current, ...value, searchFallbackEnabled: Boolean(value.searchFallbackEnabled ?? current.searchFallbackEnabled) })); }).catch(e => { if (!disposed) setSearchError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [section]);

  useEffect(() => {
    if (!nativeAvailable || section !== 'execution') return;
    let disposed = false;
    setBusy(true);
    api.hasDaytonaKey().then(value => { if (!disposed) setHasCloudKey(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.getExecutionConfig().then(value => { if (!disposed) setConfig(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    api.isolationGet().then(value => { if (!disposed) setIsolation(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    api.isolationStatus().then(value => { if (!disposed) setIsolationStatus(value); }).catch(() => {});
    return () => { disposed = true; };
  }, [section]);

  useEffect(() => {
    if (!nativeAvailable || section !== 'execution') return;
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
  }, [cloudRevision, section]);

  function updatePath(key: keyof ExecutionConfig, value: string) {
    setNotice('');
    setConfig(current => ({ ...current, [key]: value }));
    setTestResults(current => Object.fromEntries(Object.entries(current).filter(([name]) => name !== key)));
  }

  async function browse(key: keyof ExecutionConfig) {
    try {
      const path = await open({ multiple: false, filters: [{ name: 'Executable', extensions: ['exe'] }] });
      if (typeof path === 'string') updatePath(key, path);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function testInterpreter(key: keyof ExecutionConfig) {
    const path = config[key];
    if (!path.trim()) return;
    setTestResults(current => ({ ...current, [key]: { status: 'testing', message: 'Testing interpreter…' } }));
    try {
      const res = await api.testInterpreter(path);
      if (latestConfig.current[key] === path) setTestResults(current => ({ ...current, [key]: { status: 'success', message: res } }));
    } catch (e) {
      if (latestConfig.current[key] === path) setTestResults(current => ({ ...current, [key]: { status: 'error', message: errorMessage(e) } }));
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
    try {
      await api.saveExecutionConfig(config);
      if (isolation) {
        await api.isolationSave(isolation);
        setIsolationStatus(await api.isolationStatus());
      }
      setNotice('Execution settings saved.');
    }
    catch (e) { setError(errorMessage(e)); }
    finally { setBusy(false); }
  }

  async function saveSearch() {
    setSearchBusy(true); setSearchError(''); setSearchNotice('');
    try {
      await api.saveWebSearchConfig(webSearch, googleApiKey, clearGoogleApiKey);
      setWebSearch(await api.getWebSearchConfig()); setGoogleApiKey(''); setClearGoogleApiKey(false);
      setSearchNotice('Search settings saved.');
    }
    catch (e) { setSearchError(errorMessage(e)); }
    finally { setSearchBusy(false); }
  }

  async function checkSearch() {
    setSearchBusy(true); setSearchHealth('');
    try {
      const res = await api.webSearchHealth();
      let message = res.search === 'configured_not_tested' ? 'Google is configured; connection was not tested.'
        : res.search === 'unreachable' ? 'Search provider is unavailable. Check that the service is running and the saved URL is correct. Save any changes before checking again.'
        : `Search: ${String(res.search ?? 'unknown')}`;
      if (res.search === 'unreachable' && webSearch.provider === 'searxng' && webSearch.searchFallbackEnabled && res.google === 'not_configured') {
        message += ' Google fallback also needs an API key and a Custom Search Engine ID.';
      }
      setSearchHealth(message);
    } catch (e) { setSearchHealth(`Check failed: ${errorMessage(e)}`); }
    finally { setSearchBusy(false); }
  }

  if (section === 'web-search') return (
    <div className="settings-page">
      <SettingsHeader title="Web search" description="The search provider used for research and live answers." />
      <form onSubmit={event => { event.preventDefault(); void saveSearch(); }}>
        <SettingsGroup title="Provider" description="Connect to an existing SearXNG service, or configure Google Custom Search. LocalLM does not install or manage the search service.">
          <SettingRow label="Search provider" htmlFor="search-provider">
            <select
              id="search-provider"
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
          </SettingRow>
          {webSearch.provider === 'searxng' ? (
            <SettingRow label="SearXNG base URL" stacked>
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
            </SettingRow>
          ) : (
            <>
              <SettingRow label="Google API key" stacked>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  aria-label="Google API key"
                  value={googleApiKey}
                  placeholder={webSearch.hasGoogleApiKey ? 'Saved key · leave blank to keep' : 'Enter API key'}
                  disabled={!nativeAvailable || searchBusy}
                  onChange={event => {
                    setSearchNotice('');
                    setGoogleApiKey(event.target.value); setClearGoogleApiKey(false);
                  }}
                />
                {webSearch.hasGoogleApiKey && <label><input type="checkbox" checked={clearGoogleApiKey}
                  disabled={!nativeAvailable || searchBusy} onChange={event => { setClearGoogleApiKey(event.target.checked); setGoogleApiKey(''); }} /> Clear saved Google API key on save</label>}
              </SettingRow>
              <SettingRow label="Google Custom Search Engine ID" stacked>
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
              </SettingRow>
            </>
          )}
          <SettingRow label="Fallback provider" description="Use the other provider when the primary returns no results. Google fallback is capped at 90 calls/day.">
            <input
              type="checkbox"
              className="switch"
              aria-label="Fall back to the other search provider on empty results"
              checked={Boolean(webSearch.searchFallbackEnabled)}
              disabled={!nativeAvailable || searchBusy}
              onChange={event => {
                setSearchNotice('');
                setWebSearch(current => ({ ...current, searchFallbackEnabled: event.target.checked }));
              }}
            />
          </SettingRow>
        </SettingsGroup>
        <div className="settings-actions">
          <button
            type="button"
            className="secondary"
            title="Checks saved settings. Save changes first."
            disabled={!nativeAvailable || searchBusy}
            onClick={() => void checkSearch()}
          >
            Check connection
          </button>
          <button className="primary" disabled={!nativeAvailable || searchBusy}>
            {searchBusy ? 'Saving…' : 'Save search settings'}
          </button>
        </div>
        {searchHealth && <p role="status" className="settings-note">{searchHealth}</p>}
        {searchError && <p role="alert" className="error-banner">{searchError}</p>}
        {searchNotice && <p role="status" className="success">{searchNotice}</p>}
      </form>
    </div>
  );

  return (
    <div className="settings-page">
      <SettingsHeader title="Execution" description="Run code locally with your installed interpreter. Commands are unsandboxed by default; an enforced sandbox is available below." />
      <p className="settings-callout">
        {isolation?.mode === 'enforced'
          ? 'Enforced sandbox: commands run in a Windows AppContainer confined to the workspace. Kernel vulnerabilities are out of scope; this is a software boundary, not a VM.'
          : 'Local execution runs with your Windows account’s permissions, including network and filesystem access. It is not a sandbox.'}{' '}
        Ask and Auto-approve reads require approval of the complete code. Full access runs enabled code tools without prompts.
      </p>
      <form onSubmit={event => { event.preventDefault(); void save(); }}>
        <SettingsGroup title="Interpreters" description="Runs have a 90-second maximum and capture up to 64 KiB from each output stream. Enable Local code under Tools to use them.">
          {([['pythonPath', 'Python executable'], ['nodePath', 'Node.js executable'], ['powershellPath', 'PowerShell executable']] as const).map(([key, label]) => (
            <SettingRow key={key} label={label} stacked description={testResults[key]?.message ? <span className={`test-feedback ${testResults[key].status}`}>{testResults[key].message}</span> : undefined}>
              <div className="execution-path">
                <input
                  aria-label={label}
                  value={config[key]}
                  placeholder="Not configured"
                  disabled={busy}
                  spellCheck={false}
                  onChange={event => updatePath(key, event.target.value)}
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
            </SettingRow>
          ))}
        </SettingsGroup>
        {isolation && (
          <SettingsGroup
            title="Sandbox"
            description={isolationStatus && !isolationStatus.available
              ? isolationStatus.reason
              : 'Isolate spawned commands in a Windows AppContainer confined to the workspace. If the container cannot be created, enforced commands refuse to run — they never run unsandboxed.'}
          >
            <SettingRow label="Isolation mode" description="Applies to local code runs, the agent terminal, and checks — not to built-in file tools, which stay inside the workspace regardless.">
              <select
                aria-label="Isolation mode"
                value={isolation.mode}
                disabled={busy || (isolationStatus !== null && !isolationStatus.available)}
                onChange={event => {
                  setNotice('');
                  setIsolation(current => current && { ...current, mode: event.target.value as IsolationSettings['mode'] });
                }}
              >
                <option value="unsandboxed">Unsandboxed — full account permissions</option>
                <option value="enforced">Enforced — AppContainer sandbox</option>
              </select>
            </SettingRow>
            {isolation.mode === 'enforced' && (
              <>
                <SettingRow label="Network access" description="Allow sandboxed commands to reach the internet. Off blocks all networking — including loopback — and disables the agent's browser, web search, web fetch, and cloud tools too.">
                  <input
                    type="checkbox"
                    className="switch"
                    aria-label="Allow network in the sandbox"
                    checked={isolation.network}
                    disabled={busy}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, network: event.target.checked, lan: event.target.checked && current.lan });
                    }}
                  />
                </SettingRow>
                <SettingRow label="LAN access" description="Also allow the local network. Off permits internet access only.">
                  <input
                    type="checkbox"
                    className="switch"
                    aria-label="Allow LAN in the sandbox"
                    checked={isolation.lan}
                    disabled={busy || !isolation.network}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, lan: event.target.checked });
                    }}
                  />
                </SettingRow>
                <SettingRow label="LPAC" description="Stricter container (Less Privileged AppContainer). Some interpreters cannot read their registry keys and may fail.">
                  <input
                    type="checkbox"
                    className="switch"
                    aria-label="Use LPAC"
                    checked={isolation.lpac}
                    disabled={busy}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, lpac: event.target.checked });
                    }}
                  />
                </SettingRow>
                <SettingRow label="Toolchain directories" stacked description="Extra directories the container can read and execute — one per line. The spawn target's own folder is granted automatically.">
                  <textarea
                    aria-label="Toolchain directories"
                    rows={3}
                    spellCheck={false}
                    value={isolation.toolchainDirs.join('\n')}
                    disabled={busy}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, toolchainDirs: event.target.value.split('\n').map(line => line.trim()).filter(Boolean) });
                    }}
                  />
                </SettingRow>
                <SettingRow label="Process limit" description="Maximum processes inside the container.">
                  <input
                    type="number"
                    min={1}
                    max={512}
                    aria-label="Process limit"
                    value={isolation.processLimit}
                    disabled={busy}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, processLimit: Math.max(1, Number(event.target.value) || 1) });
                    }}
                  />
                </SettingRow>
                <SettingRow label="Memory limit (MB)" description="Total commit for all processes in the container.">
                  <input
                    type="number"
                    min={64}
                    step={256}
                    aria-label="Memory limit in MB"
                    value={isolation.memoryMb}
                    disabled={busy}
                    onChange={event => {
                      setNotice('');
                      setIsolation(current => current && { ...current, memoryMb: Math.max(64, Number(event.target.value) || 64) });
                    }}
                  />
                </SettingRow>
              </>
            )}
            {isolationStatus && (isolationStatus.grants?.length ?? 0) > 0 && (
              <p className="settings-note">{isolationStatus.grants.length} folder {isolationStatus.grants.length === 1 ? 'grant' : 'grants'} recorded. Leaving enforced mode revokes them.</p>
            )}
          </SettingsGroup>
        )}
        <div className="settings-actions">
          <button
            type="button"
            className="secondary"
            disabled={!nativeAvailable || busy}
            onClick={() => void autoDetect()}
          >
            Auto-detect interpreters
          </button>
          <button className="primary" disabled={!nativeAvailable || busy}>
            {busy ? 'Saving…' : 'Save local execution'}
          </button>
        </div>
      </form>
      {error && <p role="alert" className="error-banner">{error}</p>}
      {cloudError && <p role="alert" className="error-banner">Cloud cleanup status could not be refreshed: {cloudError}</p>}
      {notice && <p role="status" className="success">{notice}</p>}
      {(hasCloudKey || pendingCloud.length > 0) && <form onSubmit={event => { event.preventDefault(); void cloudAction('save'); }}>
        <SettingsGroup title="Daytona credentials" description="Cloud execution is unavailable. Credentials can be retained or updated to clean up historical cloud operations.">
          <SettingRow label="Daytona API key" stacked description={`${hasCloudKey ? 'An encrypted API key is saved.' : 'No Daytona key saved.'} Saving a key does not create a sandbox or verify account access.`}>
            <div className="execution-path">
              <input
                type="password"
                autoComplete="off"
                spellCheck={false}
                aria-label="Daytona API key"
                value={cloudKey}
                disabled={!nativeAvailable || cloudBusy}
                onChange={event => setCloudKey(event.target.value)}
              />
              <button className="secondary" disabled={!nativeAvailable || cloudBusy || !cloudKey}>Save Daytona key</button>
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
          </SettingRow>
        </SettingsGroup>
      </form>}
      {pendingCloud.length > 0 && (
        <section aria-label="Pending cloud cleanup" className="settings-group">
          <h2>Cloud cleanup needs attention</h2>
          <p className="settings-group-description">These operations may still have cloud resources. Their records are retained until removal is verified.</p>
          <div className="settings-card">
            {pendingCloud.map(item => (
              <SettingRow key={item.name} label={item.name} description={<>
                <span>{item.sandboxId ? `Sandbox: ${item.sandboxId}` : 'Creation outcome unknown; look up this operation name before retrying.'}</span>
                {item.cleanupError && <span className="setting-row-error">{item.cleanupError}</span>}
              </>}>
                <button
                  className="secondary"
                  disabled={!hasCloudKey || cloudBusy}
                  onClick={() => void cloudAction('cleanup', item.name)}
                >
                  Retry cleanup
                </button>
              </SettingRow>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
