import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { ExecutionConfig, ExecutionPolicy } from '../lib/types';

const defaults: ExecutionPolicy = { enabled: true, timeoutSeconds: 60, workingDirectory: '', environment: {}, alwaysAsk: false };

export function NativeExecutionSettings({ kind }: { kind: 'bash' | 'python' }) {
  const [config, setConfig] = useState<ExecutionConfig | null>(null);
  const [environment, setEnvironment] = useState('{}');
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const name = kind === 'bash' ? 'Bash & shell' : 'Python';
  const pathKey = kind === 'bash' ? 'bashPath' : 'pythonPath';
  const policy = { ...defaults, ...config?.[kind] };
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getExecutionConfig().then(value => {
      if (!disposed) { setConfig(value); setEnvironment(JSON.stringify(value[kind]?.environment || {}, null, 2)); }
    }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, [kind]);
  function change(value: Partial<ExecutionPolicy>) {
    setNotice(''); setConfig(current => current && ({ ...current, [kind]: { ...defaults, ...current[kind], ...value } }));
  }
  async function action(run: () => Promise<void>) {
    setBusy(true); setError(''); setNotice('');
    try { await run(); } catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  async function save() {
    if (!config) return;
    const values: unknown = JSON.parse(environment);
    if (!values || typeof values !== 'object' || Array.isArray(values) || !Object.values(values).every(v => typeof v === 'string')) throw new Error('Environment must be a JSON object with string values.');
    const next = { ...config, [kind]: { ...policy, environment: values as Record<string, string> } };
    await api.saveExecutionConfig(next); setConfig(next); setNotice(`${name} settings saved. New tool calls use these settings.`);
  }
  return <div className="settings-page">
    <div className="page-heading"><h1>{name}</h1><p>{kind === 'bash' ? 'Run commands with your selected native shell.' : 'Run Python snippets and project scripts with your own interpreter and dependencies.'}</p></div>
    {!nativeAvailable && <p className="catalog-notice">Open the desktop application to configure native execution.</p>}
    <p className="catalog-notice">Commands run with your account’s filesystem and network permissions — workspace checks are not an operating-system sandbox. Every call follows the conversation’s tool selection and permissions.</p>
    <form onSubmit={event => { event.preventDefault(); void action(save); }}>
      <fieldset disabled={busy || !nativeAvailable || !config}>
        <section className="preference-section" aria-labelledby={`${kind}-policy-heading`}>
          <div className="preference-section-heading"><h2 id={`${kind}-policy-heading`}>Runs</h2></div>
          <div className="preference-group">
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <label htmlFor={`${kind}-enabled`}>{kind === 'bash' ? 'Enable shell commands' : 'Enable Python execution'}</label>
                <p>Make the tool available when it’s selected for a conversation.</p>
              </div>
              <input id={`${kind}-enabled`} className="switch" type="checkbox" aria-label={kind === 'bash' ? 'Enable shell commands' : 'Enable Python execution'} checked={policy.enabled} onChange={event => change({ enabled: event.target.checked })} />
            </div>
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <label htmlFor={`${kind}-always-ask`}>Always request approval</label>
                <p>Ask before every run, including in Full access.</p>
              </div>
              <input id={`${kind}-always-ask`} className="switch" type="checkbox" aria-label="Always request approval, including in Full access" checked={policy.alwaysAsk} onChange={event => change({ alwaysAsk: event.target.checked })} />
            </div>
            <div className="settings-note"><p>Ask and Auto-approve reads already require approval. The interactive terminal is controlled directly by you in its own session.</p></div>
          </div>
        </section>
        <section className="preference-section" aria-labelledby={`${kind}-paths-heading`}>
          <div className="preference-section-heading"><h2 id={`${kind}-paths-heading`}>Interpreter</h2></div>
          <div className="preference-group">
            <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor={`${kind}-path`}>{kind === 'bash' ? 'Shell executable' : 'Python executable'}</label>
              </div>
              <div className="field-row">
                <input id={`${kind}-path`} aria-label={kind === 'bash' ? 'Shell executable' : 'Python executable'} value={config?.[pathKey] || ''} placeholder="Not detected" onChange={event => { setNotice(''); setConfig(current => current && ({ ...current, [pathKey]: event.target.value })); }} />
                <button type="button" className="secondary" onClick={() => void action(async () => { const detected = await api.detectInterpreters(); setConfig(current => current && ({ ...current, [pathKey]: detected[pathKey] || '' })); setNotice(detected[pathKey] ? 'Executable detected. Save to apply.' : 'No executable found on PATH. Choose an installed executable.'); })}>Detect</button>
                <button type="button" className="secondary" onClick={() => void action(async () => { const selected = await open({ multiple: false }); if (typeof selected === 'string') setConfig(current => current && ({ ...current, [pathKey]: selected })); })}>Browse</button>
                <button type="button" className="secondary" disabled={!config?.[pathKey]} onClick={() => void action(async () => { setNotice(await api.testInterpreter(config?.[pathKey] || '')); })}>Test</button>
              </div>
            </div>
            {kind === 'python' && <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor="python-venv">Virtual environment folder</label>
                <p>Uses bin/python on Linux or Scripts/python.exe on Windows. Existing packages are reused; nothing is installed automatically.</p>
              </div>
              <input id="python-venv" aria-label="Virtual environment folder" value={config?.pythonVenv || ''} placeholder="Optional: /project/.venv" onChange={event => { setNotice(''); setConfig(current => current && ({ ...current, pythonVenv: event.target.value })); }} />
            </div>}
            <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor={`${kind}-workdir`}>Default working directory</label>
                <p>Relative to the workspace, or an absolute directory inside it.</p>
              </div>
              <input id={`${kind}-workdir`} aria-label="Default working directory" value={policy.workingDirectory} placeholder="Authorized conversation workspace" onChange={event => change({ workingDirectory: event.target.value })} />
            </div>
            <div className="preference-row preference-row-block">
              <div className="field-pair">
                <label>Timeout (seconds)<input aria-label="Timeout (seconds)" type="number" min={1} max={kind === 'bash' ? 300 : 90} required value={policy.timeoutSeconds} onChange={event => change({ timeoutSeconds: Number(event.target.value) })} /></label>
                <label>Memory limit (MiB)<input aria-label="Memory limit (MiB)" type="number" min={64} max={65536} placeholder="No limit" value={policy.memoryLimitMib ?? ''} onChange={event => change({ memoryLimitMib: event.target.value ? Number(event.target.value) : null })} /></label>
              </div>
              <p className="preference-desc">Memory is a per-process Unix limit inherited by children — leave empty on Windows. The model can request shorter timeouts.</p>
            </div>
          </div>
        </section>
        <section className="preference-section" aria-labelledby={`${kind}-env-heading`}>
          <div className="preference-section-heading"><h2 id={`${kind}-env-heading`}>Environment</h2></div>
          <div className="preference-group">
            <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor={`${kind}-environment`}>Environment variables</label>
                <p>JSON string values, for example {`{"PROJECT_MODE":"test"}`}. Stored on this device — use the credential vault for secrets.</p>
              </div>
              <textarea id={`${kind}-environment`} aria-label="Environment variables" rows={5} spellCheck={false} value={environment} onChange={event => { setNotice(''); setEnvironment(event.target.value); }} />
            </div>
            <div className="connector-actions">
              <button className="primary">{busy ? 'Saving…' : `Save ${kind} settings`}</button>
            </div>
          </div>
        </section>
      </fieldset>
    </form>
    {error && <p className="error-banner" role="alert">{error}</p>}
    {notice && <p role="status">{notice}</p>}
  </div>;
}
