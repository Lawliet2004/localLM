import './AutomationSettings.css';
import { useEffect, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AutomationConfig } from '../lib/types';

const defaults: AutomationConfig = { browserEnabled: false, computerEnabled: false, browserPackagePath: '', browserExecutablePath: '', browserChannel: 'chrome', headless: false };
type Action = 'save' | 'install' | 'check' | 'reset';
type Diagnostics = Awaited<ReturnType<typeof api.automationDiagnostics>>;

export function AutomationSettings({ mode, onConfigureExecution }: { mode: 'browser' | 'computer'; onConfigureExecution?: () => void }) {
  const [config, setConfig] = useState(defaults);
  const [busy, setBusy] = useState<Action | null>(null);
  const [loading, setLoading] = useState(nativeAvailable);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [diagnostics, setDiagnostics] = useState<Diagnostics | null>(null);
  const browser = mode === 'browser';
  const title = browser ? 'Browser Use' : 'Computer Use';

  useEffect(() => {
    let disposed = false;
    if (!nativeAvailable) return;
    api.getAutomationConfig().then(value => {
      if (!disposed) { setConfig(value); setLoaded(true); }
    }).catch(e => {
      if (!disposed) setError(errorMessage(e));
    }).finally(() => {
      if (!disposed) setLoading(false);
    });
    return () => { disposed = true; };
  }, []);

  function update(patch: Partial<AutomationConfig>) {
    setConfig(current => ({ ...current, ...patch }));
    setDiagnostics(null);
    setNotice('');
  }

  async function action(kind: Action) {
    setBusy(kind); setError(''); setNotice(''); setDiagnostics(null);
    try {
      if (kind === 'save') {
        await api.saveAutomationConfig(config);
        setNotice(`${title} settings saved. Select ${browser ? 'Browser use' : 'Computer use'} in your chat’s Tools panel.`);
      }
      if (kind === 'install') {
        const installed = await api.installBrowserEngine();
        setConfig(current => ({ ...current, browserPackagePath: installed.browserPackagePath }));
        setNotice('Browser engine installed. Enable Browser use, then save settings. Chrome or Edge must be installed separately.');
      }
      if (kind === 'check') setDiagnostics(await api.automationDiagnostics(config));
      if (kind === 'reset') {
        await api.resetAutomation();
        setNotice('Browser sessions closed. The next approved browser action starts a fresh session.');
      }
    } catch (e) { setError(errorMessage(e)); }
    finally { setBusy(null); }
  }

  async function browse(field: 'browserPackagePath' | 'browserExecutablePath') {
    try {
      const path = await open({ directory: field === 'browserPackagePath', multiple: false, title: field === 'browserPackagePath' ? 'Choose @playwright/mcp package folder' : 'Choose browser executable' });
      if (typeof path === 'string') update({ [field]: path });
    } catch (e) { setError(errorMessage(e)); }
  }

  const unavailable = !nativeAvailable || !loaded || loading || busy !== null;
  return <div className="settings-page automation-settings" aria-labelledby="automation-heading">
    <div className="page-heading">
      <h1 id="automation-heading">{title}</h1>
      <p>{browser ? 'Let your assistant navigate websites in a separate browser.' : 'Let your assistant work with desktop apps on your primary monitor.'} Enable it here, then select it in your chat’s Tools panel.</p>
    </div>
    {!nativeAvailable && <p className="automation-notice" role="status">Setup and automation require the LocalLM desktop app.</p>}
    {loading && <p role="status">Loading {browser ? 'browser' : 'computer'} settings…</p>}
    <form aria-busy={loading || busy !== null} onSubmit={event => { event.preventDefault(); if (!unavailable) void action('save'); }}>
      <section className="preference-section" aria-labelledby="automation-control-heading">
        <div className="preference-section-heading"><h2 id="automation-control-heading">{browser ? 'Browser control' : 'Desktop control'}</h2></div>
        <div className="preference-group">
          <div className="preference-row preference-toggle">
            <div className="preference-copy">
              <label htmlFor="automation-enabled">Enable {browser ? 'browser' : 'computer'} use</label>
              <p id="automation-enabled-description">{browser ? 'Navigate, fill forms, click, scroll, switch tabs, and take screenshots. Each conversation has an isolated browser profile.' : 'Take screenshots, move and click the mouse, type, press keys, and scroll. Desktop actions affect the focused app.'}</p>
            </div>
            <input id="automation-enabled" className="switch" type="checkbox" aria-describedby="automation-enabled-description" checked={browser ? config.browserEnabled : config.computerEnabled} disabled={unavailable} onChange={e => update(browser ? { browserEnabled: e.target.checked } : { computerEnabled: e.target.checked })} />
          </div>
        </div>
        <p className="preference-footnote">Ask and Auto-approve still confirm every automation action, including screenshots.</p>
      </section>

      {browser ? <>
        <section className="preference-section" aria-labelledby="browser-engine-heading">
          <div className="preference-section-heading"><h2 id="browser-engine-heading">Browser engine</h2><p>Install the engine, then choose Chrome or Microsoft Edge.</p></div>
          <div className="preference-group">
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <span className="preference-title">Browser engine</span>
                <span className="preference-desc">Installs Playwright MCP 0.0.83 with npm and the Node.js executable from Execution settings. Chrome or Edge must be installed separately.</span>
              </div>
              <button type="button" className="secondary" disabled={unavailable} onClick={() => void action('install')}>{busy === 'install' ? 'Installing…' : 'Set up browser engine'}</button>
            </div>
            {onConfigureExecution && <button type="button" className="preference-row preference-nav" aria-labelledby="automation-node-title" aria-describedby="automation-node-desc" onClick={onConfigureExecution} disabled={busy !== null}>
              <span className="preference-copy">
                <span className="preference-title" id="automation-node-title">Configure Node.js</span>
                <span className="preference-desc" id="automation-node-desc">Set the Node.js executable in Execution settings.</span>
              </span>
              <ChevronRight size={17} className="preference-nav-chevron" />
            </button>}
          </div>
        </section>
        <section className="preference-section" aria-labelledby="browser-preferences-heading">
          <div className="preference-section-heading"><h2 id="browser-preferences-heading">Browser preferences</h2></div>
          <div className="preference-group">
            <div className="preference-row"><div className="preference-copy"><label htmlFor="browser-channel">Browser</label><p>Choose the browser used for new sessions.</p></div><select id="browser-channel" value={config.browserChannel} disabled={unavailable} onChange={e => update({ browserChannel: e.target.value as AutomationConfig['browserChannel'] })}><option value="chrome">Google Chrome</option><option value="msedge">Microsoft Edge</option></select></div>
            <div className="preference-row preference-toggle"><div className="preference-copy"><label htmlFor="browser-headless">Run browser in the background</label><p>Run without a visible browser window.</p></div><input id="browser-headless" className="switch" type="checkbox" checked={config.headless} disabled={unavailable} onChange={e => update({ headless: e.target.checked })} /></div>
            <details className="automation-paths"><summary>Browser paths</summary>
              <div className="settings-details-body">
                <label htmlFor="browser-package-path">Browser engine package folder</label>
                <div className="execution-path"><input id="browser-package-path" value={config.browserPackagePath} disabled={unavailable} placeholder="Set automatically by setup" onChange={e => update({ browserPackagePath: e.target.value })} /><button type="button" className="secondary" disabled={unavailable} onClick={() => void browse('browserPackagePath')}>Browse</button></div>
                <label htmlFor="browser-executable-path">Custom browser executable</label>
                <div className="execution-path"><input id="browser-executable-path" value={config.browserExecutablePath} disabled={unavailable} placeholder="Optional; uses the selected browser" onChange={e => update({ browserExecutablePath: e.target.value })} /><button type="button" className="secondary" disabled={unavailable} onClick={() => void browse('browserExecutablePath')}>Browse</button></div>
              </div>
            </details>
          </div>
        </section>
      </> : <section className="preference-section" aria-labelledby="computer-requirements-heading">
        <div className="preference-section-heading"><h2 id="computer-requirements-heading">Requirements</h2></div>
        <div className="preference-group">
          <div className="preference-row"><div className="preference-copy"><span className="preference-title">Image input</span><span className="preference-desc">Choose a model that supports image input. Local models need a vision projector; provider models need image input configured.</span></div></div>
          <div className="preference-row"><div className="preference-copy"><span className="preference-title">Desktop compatibility</span><span className="preference-desc">Desktop input supports Windows and Linux X11. Wayland input is unavailable.</span></div></div>
        </div>
      </section>}

      <div className="connector-actions automation-actions">
        <button className="primary" disabled={unavailable}>{busy === 'save' ? 'Saving…' : `Save ${browser ? 'browser' : 'computer'} settings`}</button>
        <button type="button" className="secondary" disabled={unavailable} onClick={() => void action('check')}>{busy === 'check' ? 'Checking setup…' : `Check ${browser ? 'browser' : 'computer'} setup`}</button>
        {browser && <button type="button" className="secondary" disabled={unavailable} onClick={() => void action('reset')}>{busy === 'reset' ? 'Resetting sessions…' : 'Reset browser sessions'}</button>}
      </div>
    </form>
    {error && <p role="alert" className="error-banner">{error}</p>}
    {notice && <p role="status">{notice}</p>}
    {diagnostics && <div className="automation-diagnostics" role="status">
      {browser ? diagnostics.package && diagnostics.node && diagnostics.selectedBrowser ? <>
        <p>{diagnostics.browser.available ? 'Browser prerequisites found.' : 'Browser setup needs attention.'}</p>
        <dl>
          <dt>Browser engine</dt><dd>{diagnostics.package.message}</dd>
          <dt>Node.js</dt><dd>{diagnostics.node.message}</dd>
          <dt>Selected browser</dt><dd>{diagnostics.selectedBrowser.message}</dd>
        </dl>
      </> : <p>{diagnostics.browser.message}</p> : <>
        <p>Computer setup check complete. Expand desktop diagnostics for input and screen capture availability.</p>
        <details><summary>Desktop diagnostics</summary><pre>{JSON.stringify(diagnostics.computer, null, 2)}</pre></details>
      </>}
    </div>}
  </div>;
}
