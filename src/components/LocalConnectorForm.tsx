import { useRef, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { LocalServerConfig } from '../lib/api';

export function LocalConnectorForm({ onSaved, initial, onCancel }: { onSaved: () => Promise<void>; initial?: LocalServerConfig; onCancel?: () => void }) {
  const [name, setName] = useState(initial?.name ?? '');
  const [executable, setExecutable] = useState(initial?.executable ?? '');
  const [directory, setDirectory] = useState(initial?.workingDirectory ?? '');
  const [args, setArgs] = useState(JSON.stringify(initial?.arguments ?? []));
  const [environment, setEnvironment] = useState(JSON.stringify(initial?.environment ?? {}));
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const submitting = useRef(false);
  async function save() {
    if (submitting.current) return;
    submitting.current = true; setPending(true); setError(''); setSaved(false);
    try {
      let argumentsValue: unknown; let environmentValue: unknown;
      try { argumentsValue = JSON.parse(args); environmentValue = JSON.parse(environment); }
      catch { throw new Error('Arguments and environment must contain valid JSON.'); }
      if (!Array.isArray(argumentsValue) || !argumentsValue.every(value => typeof value === 'string')) throw new Error('Arguments must be a JSON array of strings.');
      if (!environmentValue || Array.isArray(environmentValue) || typeof environmentValue !== 'object' || !Object.values(environmentValue).every(value => typeof value === 'string')) throw new Error('Environment must be a JSON object with string values.');
      await api.saveLocalConnector({ id: initial?.id ?? `local-${crypto.randomUUID()}`, name: name.trim(), executable, workingDirectory: directory, arguments: argumentsValue, environment: environmentValue as Record<string, string> });
      setName(''); setExecutable(''); setDirectory(''); setArgs('[]'); setEnvironment('{}'); setSaved(true);
      await onSaved();
    } catch (e) { setError(errorMessage(e)); }
    finally { submitting.current = false; setPending(false); }
  }
  return <details className="catalog-item" open={initial ? true : undefined}><summary>{initial ? `Edit ${initial.name}` : 'Add a local MCP server'}</summary><div className="catalog-detail">
    <p>Configure an installed MCP server. Save stores its configuration encrypted on this device. Connect launches it with your operating-system permissions, including file and network access.</p>
    <form onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={!nativeAvailable || pending}>
        <label>Name<input required maxLength={160} value={name} onChange={event => setName(event.target.value)} /></label>
        <label>Executable path<input required value={executable} onChange={event => setExecutable(event.target.value)} placeholder="Absolute path to node.exe, python.exe, or a server executable" /></label>
        <label>Working directory<input required value={directory} onChange={event => setDirectory(event.target.value)} placeholder="Absolute directory path" /></label>
        <label>Arguments (JSON array)<textarea aria-label="Arguments (JSON array)" spellCheck={false} autoComplete="off" value={args} onChange={event => setArgs(event.target.value)} /></label>
        <label>Environment variables (JSON object)<textarea aria-label="Environment variables (JSON object)" spellCheck={false} autoComplete="off" value={environment} onChange={event => setEnvironment(event.target.value)} /></label>
        <p>Use one JSON string per argument, for example ["C:\\servers\\index.js"]. Environment values may contain credentials; they are visible while editing this form.</p>
        <button className="primary">{pending ? 'Saving…' : 'Save local server'}</button>
        {onCancel && <button type="button" className="secondary" onClick={onCancel}>Cancel editing</button>}
      </fieldset>
    </form>
    {error && <p role="alert" className="error-banner">{error}</p>}
    {saved && <p role="status">Saved. Use Connect on the server below to launch it.</p>}
  </div></details>;
}
