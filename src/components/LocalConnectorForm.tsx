import { useEffect, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { LocalServerConfig } from '../lib/api';

type ArgumentRow = { value: string };
type EnvironmentRow = { name: string; value: string; revealed: boolean };
function parseArguments(raw: string): string[] {
  const value: unknown = JSON.parse(raw);
  if (!Array.isArray(value) || !value.every(item => typeof item === 'string')) throw new Error('Arguments must be a JSON array of strings.');
  return value;
}
function parseEnvironment(raw: string): Record<string, string> {
  const value: unknown = JSON.parse(raw);
  if (!value || Array.isArray(value) || typeof value !== 'object' || !Object.values(value).every(item => typeof item === 'string')) throw new Error('Environment must be a JSON object with string values.');
  return value as Record<string, string>;
}

export function LocalConnectorForm({ onSaved, initial, onCancel }: { onSaved: () => Promise<void>; initial?: LocalServerConfig; onCancel?: () => void }) {
  const [name, setName] = useState(initial?.name ?? '');
  const [executable, setExecutable] = useState(initial?.executable ?? '');
  const [directory, setDirectory] = useState(initial?.workingDirectory ?? '');
  const [argumentRows, setArgumentRows] = useState<ArgumentRow[]>(() => (initial?.arguments ?? []).map(value => ({ value })));
  const [environmentRows, setEnvironmentRows] = useState<EnvironmentRow[]>(() => Object.entries(initial?.environment ?? {}).map(([envName, envValue]) => ({ name: envName, value: envValue, revealed: false })));
  const [advancedArguments, setAdvancedArguments] = useState(false);
  const [advancedEnvironment, setAdvancedEnvironment] = useState(false);
  const [rawArguments, setRawArguments] = useState(JSON.stringify(initial?.arguments ?? []));
  const [rawEnvironment, setRawEnvironment] = useState(JSON.stringify(initial?.environment ?? {}));
  const [argumentError, setArgumentError] = useState('');
  const [environmentError, setEnvironmentError] = useState('');
  const [choosing, setChoosing] = useState<'executable' | 'directory' | null>(null);
  const [duplicateWarning, setDuplicateWarning] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const nameInput = useRef<HTMLInputElement>(null);
  const submitting = useRef(false);
  useEffect(() => { nameInput.current?.focus(); }, []);
  useEffect(() => {
    try {
      const names = new Set<string>();
      for (const row of environmentRows) {
        const key = row.name.trim().toUpperCase();
        if (!row.name.trim()) continue;
        if (names.has(key)) { setDuplicateWarning(`Duplicate environment name: ${row.name.trim()}. Names must be unique regardless of letter case.`); return; }
        names.add(key);
      }
      setDuplicateWarning('');
    } catch { setDuplicateWarning(''); }
  }, [environmentRows]);
  function structuredArguments(): string[] {
    setArgumentError('');
    return argumentRows.map(row => row.value);
  }
  function structuredEnvironment(): Record<string, string> {
    setEnvironmentError('');
    const result: Record<string, string> = {};
    for (const row of environmentRows) {
      if (!row.name.trim() && !row.value) continue;
      result[row.name.trim()] = row.value;
    }
    return result;
  }
  async function choose(target: 'executable' | 'directory') {
    setChoosing(target); setError('');
    try {
      const selected = await open({ multiple: false, directory: target === 'directory', title: target === 'directory' ? 'Choose working directory' : 'Choose server executable' });
      if (typeof selected === 'string') {
        if (target === 'executable') setExecutable(selected); else setDirectory(selected);
      }
    } catch (e) { setError(errorMessage(e)); }
    finally { setChoosing(null); }
  }
  async function save() {
    if (submitting.current) return;
    submitting.current = true; setPending(true); setError(''); setSaved(false);
    try {
      let args: string[]; let environment: Record<string, string>;
      try {
        args = advancedArguments ? parseArguments(rawArguments) : structuredArguments();
        environment = advancedEnvironment ? parseEnvironment(rawEnvironment) : structuredEnvironment();
      } catch (e) { throw e instanceof Error ? e : new Error('Invalid arguments or environment.'); }
      if (!advancedArguments) {
        for (const [index, value] of args.entries()) {
          if (value.includes('\0')) throw new Error(`Argument ${index + 1} contains an unsupported character.`);
        }
      }
      await api.saveLocalConnector({ id: initial?.id ?? `local-${crypto.randomUUID()}`, name: name.trim(), executable, workingDirectory: directory, arguments: args, environment });
      setName(''); setExecutable(''); setDirectory(''); setArgumentRows([]); setEnvironmentRows([]); setRawArguments('[]'); setRawEnvironment('{}'); setSaved(true);
      await onSaved();
    } catch (e) { setError(errorMessage(e)); }
    finally { submitting.current = false; setPending(false); }
  }
  const argumentCount = advancedArguments ? 'JSON mode' : `${argumentRows.length} argument${argumentRows.length === 1 ? '' : 's'}`;
  const environmentCount = advancedEnvironment ? 'JSON mode' : `${environmentRows.length} variable${environmentRows.length === 1 ? '' : 's'}`;
  return <details className="catalog-item local-connector-form" open={initial ? true : undefined}><summary>{initial ? `Edit ${initial.name}` : 'Add a local MCP server'}</summary><div className="catalog-detail">
    <p>Configure an installed MCP server. Save stores its configuration encrypted on this device without launching it. Connect launches it with your operating-system permissions, including file and network access.</p>
    <form onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={!nativeAvailable || pending}>
        <label>Name<input ref={nameInput} aria-label="Name" required maxLength={160} value={name} onChange={event => setName(event.target.value)} placeholder="My local server" /></label>
        {duplicateWarning && <p role="alert" className="error-banner">{duplicateWarning}</p>}
        <label>Executable path<div className="field-row"><input aria-label="Executable path" required value={executable} onChange={event => setExecutable(event.target.value)} placeholder="Absolute path to node.exe, python.exe, or a server executable" spellCheck={false} /><button type="button" className="secondary" disabled={!nativeAvailable || pending || choosing !== null} onClick={() => void choose('executable')}>{choosing === 'executable' ? 'Choosing…' : 'Browse…'}</button></div></label>
        <label>Working directory<div className="field-row"><input aria-label="Working directory" required value={directory} onChange={event => setDirectory(event.target.value)} placeholder="Absolute directory path" spellCheck={false} /><button type="button" className="secondary" disabled={!nativeAvailable || pending || choosing !== null} onClick={() => void choose('directory')}>{choosing === 'directory' ? 'Choosing…' : 'Browse…'}</button></div></label>
        <fieldset className="structured-group"><legend>Arguments · {argumentCount}</legend>
          {!advancedArguments && <>
            {argumentRows.map((row, index) => <div className="field-row" key={index}><input aria-label={`Argument ${index + 1}`} value={row.value} spellCheck={false} autoComplete="off" placeholder={index === 0 ? 'C:\\servers\\index.js' : `Argument ${index + 1} (empty allowed)`} onChange={event => setArgumentRows(current => current.map((item, position) => position === index ? { value: event.target.value } : item))} /><button type="button" className="secondary" aria-label={`Remove argument ${index + 1}`} onClick={() => setArgumentRows(current => current.filter((_, position) => position !== index))}>Remove</button></div>)}
            <div className="connector-actions"><button type="button" className="secondary" onClick={() => setArgumentRows(current => [...current, { value: '' }])}>Add argument</button><button type="button" className="secondary" onClick={() => { setRawArguments(JSON.stringify(argumentRows.map(row => row.value))); setAdvancedArguments(true); }}>Edit arguments as JSON</button></div>
            <p><small>Each row is exactly one process argument. Empty arguments are preserved.</small></p>
          </>}
          {advancedArguments && <>
            <label>Arguments (JSON array)<textarea aria-label="Arguments (JSON array)" spellCheck={false} autoComplete="off" value={rawArguments} onChange={event => setRawArguments(event.target.value)} /></label>
            <div className="connector-actions"><button type="button" className="secondary" onClick={() => { try { setArgumentRows(parseArguments(rawArguments).map(value => ({ value }))); setArgumentError(''); setAdvancedArguments(false); } catch (e) { setArgumentError(errorMessage(e)); } }}>Use structured editor</button></div>
            {argumentError && <p role="alert" className="error-banner">{argumentError}</p>}
            <p><small>Use one JSON string per argument, for example ["C:\\servers\\index.js", ""].</small></p>
          </>}
        </fieldset>
        <fieldset className="structured-group"><legend>Environment · {environmentCount}</legend>
          {!advancedEnvironment && <>
            {environmentRows.map((row, index) => <div className="field-row env-row" key={index}><input aria-label={`Environment name ${index + 1}`} value={row.name} spellCheck={false} autoComplete="off" placeholder="TOKEN_NAME" onChange={event => setEnvironmentRows(current => current.map((item, position) => position === index ? { ...item, name: event.target.value } : item))} /><input aria-label={`Environment value ${index + 1}`} type={row.revealed ? 'text' : 'password'} value={row.value} spellCheck={false} autoComplete="off" placeholder="value (may be empty)" onChange={event => setEnvironmentRows(current => current.map((item, position) => position === index ? { ...item, value: event.target.value } : item))} /><button type="button" className="secondary" aria-label={`${row.revealed ? 'Hide' : 'Show'} environment value ${index + 1}`} onClick={() => setEnvironmentRows(current => current.map((item, position) => position === index ? { ...item, revealed: !item.revealed } : item))}>{row.revealed ? 'Hide' : 'Show'}</button><button type="button" className="secondary" aria-label={`Remove environment variable ${index + 1}`} onClick={() => setEnvironmentRows(current => current.filter((_, position) => position !== index))}>Remove</button></div>)}
            <div className="connector-actions"><button type="button" className="secondary" onClick={() => setEnvironmentRows(current => [...current, { name: '', value: '', revealed: false }])}>Add variable</button><button type="button" className="secondary" onClick={() => { setRawEnvironment(JSON.stringify(structuredEnvironment())); setAdvancedEnvironment(true); }}>Edit environment as JSON</button></div>
            <p><small>Values may contain credentials; they are hidden until revealed. Review them before saving.</small></p>
          </>}
          {advancedEnvironment && <>
            <label>Environment variables (JSON object)<textarea aria-label="Environment variables (JSON object)" spellCheck={false} autoComplete="off" value={rawEnvironment} onChange={event => setRawEnvironment(event.target.value)} /></label>
            <div className="connector-actions"><button type="button" className="secondary" onClick={() => { try { setEnvironmentRows(Object.entries(parseEnvironment(rawEnvironment)).map(([envName, envValue]) => ({ name: envName, value: envValue, revealed: false }))); setEnvironmentError(''); setAdvancedEnvironment(false); } catch (e) { setEnvironmentError(errorMessage(e)); } }}>Use structured editor</button></div>
            {environmentError && <p role="alert" className="error-banner">{environmentError}</p>}
            <p><small>Values may contain credentials; they are visible while editing this form.</small></p>
          </>}
        </fieldset>
        <p>Saved servers stay disconnected until you choose Connect. Edits keep the same stable server ID.</p>
        <div className="connector-actions"><button className="primary">{pending ? 'Saving…' : 'Save local server'}</button>
        {onCancel && <button type="button" className="secondary" onClick={onCancel}>Cancel editing</button>}</div>
      </fieldset>
    </form>
    {error && <p role="alert" className="error-banner">{error}</p>}
    {saved && <p role="status">Saved. Use Connect on the server below to launch it.</p>}
  </div></details>;
}
