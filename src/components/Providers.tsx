import { useRef, useState } from 'react';
import { Cloud, Eye, EyeOff, Plus } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { isSubscriptionFormat, LOCAL_PROVIDER_PRESETS, MAX_PROVIDER_MODELS, parseProviderBaseUrl, PROVIDER_PRESETS, providerRequiresApiKey, remoteModelReady } from '../lib/providers';
import type { ApiFormat, ModelControls, ModelSelection, ProviderConnection, ProviderDraft, RemoteModel, ToolSupport } from '../lib/types';

const EDITABLE_FORMATS: { value: ApiFormat; label: string; hint: string }[] = [
  { value: 'openai-chat-completions', label: 'OpenAI-compatible', hint: 'POST /chat/completions — llama.cpp servers, OpenAI, OpenRouter, and most hosted endpoints.' },
  { value: 'openai-responses', label: 'OpenAI Responses', hint: 'Native Responses API (POST /v1/responses) with function calls and stateless conversation history.' },
  { value: 'claude-messages', label: 'Claude', hint: 'Native Messages API (POST /v1/messages) authenticated with x-api-key.' },
];
const FORMAT_LABELS: Record<string, string> = {
  'openai-chat-completions': 'OpenAI-compatible',
  'openai-responses': 'OpenAI Responses',
  'claude-messages': 'Claude',
  'freetoken-openai': 'FreeToken',
  'chatgpt-subscription': 'ChatGPT subscription',
  'grok-subscription': 'Grok subscription',
};
const TOOL_LABELS: Record<ToolSupport, string> = { unknown: 'tools unknown', supported: 'tools', unsupported: 'no tools' };

function formatTokens(value: number): string {
  return value >= 1024 ? `${Math.round(value / 1024)}k` : String(value);
}
function parseCapacity(raw: string, min: number, max: number, label: string): number | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  if (!Number.isInteger(value) || value < min || value > max) {
    // Locale-stable grouping: the host locale (e.g. en-IN) would otherwise
    // render "2,000,000" as "20,00,000" in an error message tests assert on.
    const grouped = new Intl.NumberFormat('en-US').format;
    throw new Error(`${label} must be a whole number between ${grouped(min)} and ${grouped(max)} tokens.`);
  }
  return value;
}

interface ModelRow { key: string; id: string; context: string; maxOutput: string; toolSupport: ToolSupport; supportsImages: boolean; newlyAdded: boolean; controls?: ModelControls | null }
const UNKNOWN_CONTROLS: ModelControls = { source: 'configured', temperature: 'unknown', topP: 'unknown', reasoningEffort: 'unknown', toolChoice: 'unknown', parallelToolCalls: 'unknown', structuredOutput: 'unknown', tokenLimitField: 'protocol-default' };
const CONTROL_FIELDS = [['temperature', 'Temperature'], ['topP', 'Top p'], ['reasoningEffort', 'Reasoning effort'], ['toolChoice', 'Tool choice'], ['parallelToolCalls', 'Parallel tool control'], ['structuredOutput', 'Structured output']] as const;

function ProviderEditor({ provider, busy = false, onClose, onSaved, onRefresh }: {
  provider?: ProviderConnection;
  busy?: boolean;
  onClose: () => void;
  onSaved: (message: string) => void;
  onRefresh?: () => Promise<void>;
}) {
  const nextKey = useRef(0);
  const newRow = (model?: RemoteModel): ModelRow => ({
    key: `model-${nextKey.current++}`,
    id: model?.id ?? '',
    context: model?.contextLength?.toString() ?? '',
    maxOutput: model?.maxOutputTokens?.toString() ?? '',
    toolSupport: model?.toolSupport ?? 'unknown',
    supportsImages: model?.supportsImages ?? false,
    newlyAdded: !model,
    controls: model?.controls,
  });
  const [savedId, setSavedId] = useState(provider?.id ?? null);
  const [savedEndpoint, setSavedEndpoint] = useState(provider ? { baseUrl: provider.baseUrl, apiFormat: provider.apiFormat } : null);
  const [preset, setPreset] = useState('custom');
  const [name, setName] = useState(provider?.name ?? '');
  const [apiFormat, setApiFormat] = useState<ApiFormat>(provider?.apiFormat ?? 'openai-chat-completions');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  const [apiKey, setApiKey] = useState('');
  const [hasApiKey, setHasApiKey] = useState(provider?.hasApiKey ?? false);
  const [revealKey, setRevealKey] = useState(false);
  const [rows, setRows] = useState<ModelRow[]>(() => (provider?.models ?? []).map(newRow));
  const [probes, setProbes] = useState<Record<string, { pending: boolean; result?: { verified: boolean; supported: boolean; detail: string } }>>({});
  const [modelQuery, setModelQuery] = useState('');
  const [pending, setPending] = useState<'' | 'save' | 'test' | 'fetch'>('');
  const [formError, setFormError] = useState('');
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);
  const submitting = useRef(false);

  const parsed = parseProviderBaseUrl(baseUrl);
  const endpointChanged = savedEndpoint !== null && ('error' in parsed || parsed.normalized !== savedEndpoint.baseUrl || apiFormat !== savedEndpoint.apiFormat);
  const format = EDITABLE_FORMATS.find(item => item.value === apiFormat);
  const keyHelper = hasApiKey && endpointChanged
    ? 'Enter a key for this endpoint. The saved key belongs to the previous URL and API format.'
    : apiFormat === 'claude-messages'
    ? hasApiKey
      ? 'A key is saved — leave blank to keep it, or paste a new key to replace it. Claude endpoints always need one.'
      : 'Claude endpoints always need an API key, localhost included. Stored encrypted on this device and sent only to this endpoint.'
    : hasApiKey
      ? 'A key is saved — leave blank to keep it, or paste a new key to replace it.'
      : 'Optional for OpenAI-compatible providers — only Claude endpoints require one. Stored encrypted on this device and sent only to this endpoint.';

  function updateRow(key: string, patch: Partial<ModelRow>) {
    setRows(current => current.map(row => row.key === key ? { ...row, ...(patch.id !== undefined && patch.id !== row.id && row.controls?.source === 'provider-reported' ? { controls: undefined } : {}), ...patch } : row));
  }
  function clearReportedControls() {
    setRows(current => current.map(row => row.controls?.source === 'provider-reported' ? { ...row, controls: undefined } : row));
  }
  function buildModels(): RemoteModel[] {
    if (rows.length > MAX_PROVIDER_MODELS) throw new Error(`A provider can store at most ${MAX_PROVIDER_MODELS} model IDs.`);
    const seen = new Set<string>();
    return rows.map((row, index) => {
      const modelId = row.id.trim();
      const label = `Model ${index + 1}${modelId ? ` “${modelId}”` : ''}`;
      if (!modelId) throw new Error(`Model ${index + 1} needs an ID — remove the row or fill it in.`);
      if (modelId.length > 512 || /[\u0000-\u001f\u007f]/.test(modelId)) throw new Error(`${label} has an invalid ID.`);
      if (seen.has(modelId)) throw new Error(`Duplicate model ID “${modelId}”.`);
      if (row.controls && ((apiFormat === 'openai-responses' && !['protocol-default', 'max-output-tokens'].includes(row.controls.tokenLimitField)) || (apiFormat === 'claude-messages' && !['protocol-default', 'max-tokens'].includes(row.controls.tokenLimitField)) || (apiFormat !== 'openai-responses' && row.controls.tokenLimitField === 'max-output-tokens'))) throw new Error(`${label} output-token field does not match this API format. Choose Protocol default or a supported field in Inference controls.`);
      seen.add(modelId);
      return {
        id: modelId,
        contextLength: parseCapacity(row.context, 128, 2_000_000, `${label} context`),
        maxOutputTokens: parseCapacity(row.maxOutput, 1, 1_000_000, `${label} maximum output`),
        toolSupport: row.toolSupport,
        supportsImages: row.supportsImages,
        ...(row.controls ? { controls: row.controls } : {}),
      };
    });
  }
  function validate(): ProviderDraft {
    if ('error' in parsed) throw new Error(parsed.error);
    const trimmedName = name.trim();
    if (!trimmedName) throw new Error('Enter a provider name.');
    if (trimmedName.length > 120) throw new Error('Provider names are limited to 120 characters.');
    const key = apiKey.trim();
    if (!key && hasApiKey && endpointChanged) {
      throw new Error('Enter an API key for the new endpoint. Your saved key will not be sent to a different URL or API format.');
    }
    if (!key && !hasApiKey && providerRequiresApiKey(apiFormat, parsed.normalized)) {
      throw new Error('Claude endpoints always need an API key.');
    }
    return { id: savedId, name: trimmedName, apiFormat, baseUrl: parsed.normalized, apiKey: key || null, models: buildModels() };
  }
  /** Merge endpoint-reported models into the editable rows so a later Save
   * keeps them instead of overwriting the discovery with stale rows. */
  function mergeListed(listed: RemoteModel[]) {
    setRows(current => {
      const known = new Map(current.map(row => [row.id.trim(), row]));
      const next = [...current];
      for (const model of listed) {
        const existing = known.get(model.id);
        if (existing) {
          next[next.indexOf(existing)] = {
            ...existing,
            context: existing.context || (model.contextLength ? String(model.contextLength) : ''),
            maxOutput: existing.maxOutput || (model.maxOutputTokens ? String(model.maxOutputTokens) : ''),
            toolSupport: existing.toolSupport === 'unknown' ? model.toolSupport : existing.toolSupport,
            supportsImages: existing.context ? existing.supportsImages : model.supportsImages ?? existing.supportsImages,
            controls: existing.controls?.source === 'configured' ? existing.controls : model.controls ?? existing.controls,
          };
        } else {
          next.push(newRow(model));
        }
      }
      return next;
    });
  }

  async function runProbe(row: ModelRow, providerId: string) {
    setProbes(current => ({ ...current, [row.key]: { pending: true } }));
    try {
      const result = await api.probeModelTools(providerId, row.id);
      setProbes(current => ({ ...current, [row.key]: { pending: false, result } }));
    } catch (error) {
      setProbes(current => ({ ...current, [row.key]: { pending: false, result: { verified: false, supported: false, detail: errorMessage(error) } } }));
    }
  }

  /** Test and fetch run against the unsaved draft — nothing persists until
   * Save. Both keep the editor open and report the real result inline. */
  async function submit(action: 'save' | 'test' | 'fetch') {
    if (submitting.current || busy || !nativeAvailable) return;
    setFormError('');
    setTestResult(null);
    let draft: ProviderDraft;
    try { draft = validate(); }
    catch (e) { setFormError(errorMessage(e)); return; }
    submitting.current = true;
    setPending(action);
    if (action !== 'save') {
      try {
        if (action === 'test') {
          const result = await api.testProviderDraft(draft);
          setTestResult({ ok: result.verified, message: `${draft.name}: ${result.message}` });
          // The test reports discovered model IDs — merge them so a later
          // Save doesn't discard the discovery.
          if (result.models.length) mergeListed(result.models.map(id => ({ id, contextLength: null, maxOutputTokens: null, toolSupport: 'unknown' as ToolSupport, supportsImages: false })));
        } else {
          const listed = await api.listProviderModelsDraft(draft);
          mergeListed(listed);
          setTestResult({ ok: true, message: `${draft.name}: found ${listed.length} model${listed.length === 1 ? '' : 's'} and added them below.` });
        }
      } catch (e) {
        setTestResult({ ok: false, message: `${draft.name}: ${errorMessage(e)}` });
      }
      submitting.current = false;
      setPending('');
      return;
    }
    let saved: ProviderConnection;
    try {
      saved = await api.saveProvider(draft);
      // Clear the submitted secret promptly; keep the server-issued ID so a
      // retry edits the saved provider instead of creating a duplicate.
      setApiKey('');
      setSavedId(saved.id);
      // Freshly added models have no probe result — verify tool calling once
      // in the background; failures keep verified=false and change nothing.
      for (const row of rows.filter(r => r.newlyAdded && r.id)) void runProbe(row, saved.id);
      setHasApiKey(saved.hasApiKey);
      setSavedEndpoint({ baseUrl: saved.baseUrl, apiFormat: saved.apiFormat });
    } catch (e) {
      setFormError(errorMessage(e));
      submitting.current = false;
      setPending('');
      return;
    }
    let failure = '';
    try { await onRefresh?.(); }
    catch (e) { failure = `${saved.name} was saved, but refreshing settings failed: ${errorMessage(e)}`; }
    submitting.current = false;
    setPending('');
    if (failure) {
      // Keep the editor open so the draft can be corrected and retried — the
      // provider is already saved under `saved.id`, so retrying is an update.
      setFormError(failure);
      return;
    }
    onClose();
    onSaved(saved.verified ? `${saved.name} saved.` : `${saved.name} saved. Test the connection before selecting one of its models.`);
  }
  const visibleRows = rows.map((row, index) => ({ row, index }))
    .filter(({ row }) => row.newlyAdded || row.id.toLowerCase().includes(modelQuery.trim().toLowerCase()))
    .sort((a, b) => Number(b.row.newlyAdded) - Number(a.row.newlyAdded));

  return <section className="provider-editor" aria-label={provider ? `Edit ${provider.name}` : 'Add API provider'}>
    <h2>{provider || savedId ? `Edit ${provider?.name ?? (name || 'provider')}` : 'Add API provider'}</h2>
    <form className="runtime-form" onSubmit={event => { event.preventDefault(); void submit('save'); }}>
      <fieldset disabled={busy || pending !== ''}>
        <p className="section-description">Connect a hosted API or a local engine. Saving a provider keeps local inference available; choose a model to start using this connection.</p>
        {!savedId && <label>Quick setup
          <select aria-label="Quick setup" value={preset} onChange={event => {
            setPreset(event.target.value);
            clearReportedControls();
            const chosen = [...PROVIDER_PRESETS, ...LOCAL_PROVIDER_PRESETS].find(item => item.id === event.target.value);
            setName(chosen?.name ?? '');
            setBaseUrl(chosen?.baseUrl ?? '');
            setApiFormat(chosen?.apiFormat ?? 'openai-chat-completions');
            setApiKey('');
            setFormError('');
          }}>
            <option value="custom">Custom provider</option>
            <optgroup label="Hosted APIs">{PROVIDER_PRESETS.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</optgroup>
            <optgroup label="Local servers">{LOCAL_PROVIDER_PRESETS.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</optgroup>
          </select>
          <small>Choose a shortcut or enter any compatible endpoint below.</small>
        </label>}
        <div className="field-pair">
          <label>Name<input aria-label="Provider name" required maxLength={120} autoFocus value={name} onChange={event => setName(event.target.value)} placeholder="My provider" /></label>
          <label>API format
            <select aria-label="API format" value={apiFormat} onChange={event => { clearReportedControls(); setApiFormat(event.target.value as ApiFormat); }}>
              {EDITABLE_FORMATS.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}
              {!format && <option value={apiFormat}>{FORMAT_LABELS[apiFormat] ?? apiFormat}</option>}
            </select>
            <small>{format?.hint ?? 'This provider uses a format managed elsewhere.'}</small>
          </label>
        </div>
        <label>API base URL<input aria-label="API base URL" required spellCheck={false} autoComplete="off" value={baseUrl} onChange={event => { clearReportedControls(); setBaseUrl(event.target.value); }} placeholder="https://api.example.com/v1" />
          <small>Use the API root, including its prefix (such as /api/v1), without /chat/completions, /responses or /messages. An empty path uses /v1. Hosted APIs require HTTPS; localhost may use HTTP.</small>
        </label>
        <label>API key
          <div className="provider-key-row">
            <input aria-label="API key" type={revealKey ? 'text' : 'password'} spellCheck={false} autoComplete="off" value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={hasApiKey && !endpointChanged ? 'Saved key · enter to replace' : 'Paste your API key'} />
            <button type="button" className="secondary" aria-label={revealKey ? 'Hide API key' : 'Show API key'} aria-pressed={revealKey} onClick={() => setRevealKey(value => !value)}>{revealKey ? <EyeOff size={14} /> : <Eye size={14} />}{revealKey ? 'Hide' : 'Show'}</button>
          </div>
          <small>{keyHelper}</small>
        </label>
        <fieldset className="provider-models-edit"><legend>Models</legend>
          <p className="section-description">Fetch models asks the endpoint for its model IDs and limits — Test connection does the same. Add a model manually if needed; set its context tokens if the provider does not report them.</p>
          {rows.length > 10 && <label>Find a model<input type="search" aria-label="Find model to edit" value={modelQuery} onChange={event => setModelQuery(event.target.value)} placeholder="Search model IDs" /></label>}
          {visibleRows.slice(0, 20).map(({ row, index }) => <div className="provider-model-edit" key={row.key}>
            <div className="provider-model-head">
              <input aria-label={`Model ${index + 1} ID`} spellCheck={false} autoComplete="off" value={row.id} placeholder="model-id, for example gpt-4o-mini" onChange={event => updateRow(row.key, { id: event.target.value })} />
              <button type="button" className="secondary" aria-label={`Remove model ${index + 1}`} onClick={() => setRows(current => current.filter(item => item.key !== row.key))}>Remove</button>
            </div>
            <div className="provider-model-grid">
              <label>Context tokens<input aria-label={`Model ${index + 1} context tokens`} inputMode="numeric" value={row.context} placeholder="required to select" onChange={event => updateRow(row.key, { context: event.target.value })} /></label>
              <label>Max output<input aria-label={`Model ${index + 1} maximum output tokens`} inputMode="numeric" value={row.maxOutput} placeholder="optional" onChange={event => updateRow(row.key, { maxOutput: event.target.value })} /></label>
              <label>Tools<select aria-label={`Model ${index + 1} tool support`} value={row.toolSupport} onChange={event => updateRow(row.key, { toolSupport: event.target.value as ToolSupport })}>
                <option value="unknown">Unknown</option>
                <option value="supported">Supported</option>
                <option value="unsupported">Unsupported</option>
              </select></label>
              <label className="check-row"><input aria-label={`Model ${index + 1} image support`} type="checkbox" checked={row.supportsImages} onChange={event => updateRow(row.key, { supportsImages: event.target.checked })} />Images</label>
              <div>
                <button type="button" className="secondary" aria-label={`Test model ${index + 1} tool calling`} disabled={!savedId || !row.id || probes[row.key]?.pending} onClick={() => void runProbe(row, savedId ?? '')}>Test tool calling</button>
                {probes[row.key]?.result && <p className="provider-hint">{probes[row.key]?.result?.supported ? 'Tool calls verified' : probes[row.key]?.result?.verified ? 'Tool calls not supported' : 'Probe failed'} — {probes[row.key]?.result?.detail}</p>}
              </div>
            </div>
            <details>
              <summary>Inference controls</summary>
              <p className="provider-hint">{row.controls ? row.controls.source === 'provider-reported' ? 'Reported by the endpoint; these capabilities have not been independently tested.' : 'Your saved controls. Supported capabilities have not been independently tested.' : 'Automatic model defaults are in use. Configure controls when your endpoint needs specific options.'}</p>
              {!row.controls ? <button type="button" className="secondary" aria-label={`Configure model ${index + 1} inference controls`} onClick={() => updateRow(row.key, { controls: { ...UNKNOWN_CONTROLS } })}>Configure controls</button> : <>
                <div className="provider-model-grid">
                  {CONTROL_FIELDS.map(([field, label]) => <label key={field}>{label}<select aria-label={`Model ${index + 1} ${label.toLowerCase()} support`} value={row.controls![field]} onChange={event => updateRow(row.key, { controls: { ...row.controls!, source: 'configured', [field]: event.target.value as ToolSupport } })}>
                    <option value="unknown">Unknown</option><option value="supported">Supported</option><option value="unsupported">Unsupported</option>
                  </select></label>)}
                  <label>Output-token field<select aria-label={`Model ${index + 1} output-token field`} value={row.controls.tokenLimitField} onChange={event => updateRow(row.key, { controls: { ...row.controls!, source: 'configured', tokenLimitField: event.target.value as ModelControls['tokenLimitField'] } })}>
                    <option value="protocol-default">Protocol default</option>
                    <option value="max-tokens" disabled={apiFormat === 'openai-responses'}>max_tokens</option>
                    <option value="max-completion-tokens" disabled={apiFormat === 'openai-responses' || apiFormat === 'claude-messages'}>max_completion_tokens</option>
                    <option value="max-output-tokens" disabled={apiFormat !== 'openai-responses'}>max_output_tokens</option>
                  </select></label>
                </div>
                <button type="button" className="secondary" aria-label={`Reset model ${index + 1} inference controls`} onClick={() => updateRow(row.key, { controls: undefined })}>Use automatic mapping</button>
              </>}
            </details>
          </div>)}
          {visibleRows.length > 20 && <p className="provider-hint">Showing 20 of {visibleRows.length} models. Search to find another model.</p>}
          {rows.length > 0 && visibleRows.length === 0 && <p className="provider-hint" role="status">No models match “{modelQuery}”.</p>}
          <div className="provider-model-toolbar">
            <button type="button" className="secondary" disabled={!nativeAvailable || pending !== ''} title="Save this provider and list the models its endpoint reports" onClick={() => void submit('fetch')}>{pending === 'fetch' ? 'Fetching…' : 'Fetch models'}</button>
            <button type="button" className="secondary" disabled={rows.length >= MAX_PROVIDER_MODELS} onClick={() => { setModelQuery(''); setRows(current => [...current, newRow()]); }}>Add model ID</button>
          </div>
        </fieldset>
        {!nativeAvailable && <p className="provider-preview-note">Browser preview — open the desktop app to save and test providers.</p>}
        {testResult && <p role={testResult.ok ? 'status' : 'alert'} className={testResult.ok ? 'success' : 'error'}>{testResult.message}</p>}
        {formError && <p role="alert" className="error">{formError}</p>}
        <div className="form-footer">
          <small>{savedId ? 'Changing the endpoint or key requires a new connection test.' : 'Keys are stored encrypted on this device.'}</small>
          <div className="provider-editor-actions">
            <button type="button" className="secondary" disabled={pending !== ''} onClick={onClose}>Cancel</button>
            <button type="button" className="secondary" disabled={!nativeAvailable || pending !== ''} title="Save this provider and verify its endpoint answers" onClick={() => void submit('test')}>{pending === 'test' ? 'Testing…' : 'Test connection'}</button>
            <button type="submit" className="primary" disabled={!nativeAvailable || pending !== ''}>{pending === 'save' ? 'Saving…' : 'Save provider'}</button>
          </div>
        </div>
      </fieldset>
    </form>
  </section>;
}

export function Providers({ providers, selection, busy = false, onSaveSelection, onRefresh }: {
  providers: ProviderConnection[];
  selection: ModelSelection;
  busy?: boolean;
  onSaveSelection: (selection: ModelSelection) => Promise<void>;
  onRefresh?: () => Promise<void>;
}) {
  const custom = providers.filter(provider => !isSubscriptionFormat(provider.apiFormat));
  // The store seeds two subscription rows regardless of sign-in; show only
  // ones that were actually configured so they do not crowd the empty state.
  const subscriptions = providers.filter(provider => isSubscriptionFormat(provider.apiFormat) && (provider.hasApiKey || provider.verified));
  const [editing, setEditing] = useState<ProviderConnection | 'new' | null>(null);
  const [pending, setPending] = useState('');
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [confirmDeleteKey, setConfirmDeleteKey] = useState<string | null>(null);
  const [modelQueries, setModelQueries] = useState<Record<string, string>>({});
  const actionInFlight = useRef(false);
  const actionsDisabled = busy || !nativeAvailable || editing !== null;

  async function run(token: string, action: () => Promise<void>) {
    if (actionInFlight.current || actionsDisabled) return;
    actionInFlight.current = true;
    setPending(token);
    setError('');
    setNotice('');
    try { await action(); } catch (e) { setError(errorMessage(e)); }
    finally { actionInFlight.current = false; setPending(''); }
  }
  async function refresh() {
    try { await onRefresh?.(); } catch (e) { setError(errorMessage(e)); return false; }
    return true;
  }
  async function testConnection(provider: ProviderConnection) {
    await run(`test-${provider.id}`, async () => {
      let message = '';
      try { message = (await api.testProvider(provider.id)).message; }
      catch (e) { setError(`${provider.name}: ${errorMessage(e)}`); }
      if (await refresh() && message) setNotice(`${provider.name}: ${message}`);
    });
  }
  async function discover(provider: ProviderConnection) {
    await run(`discover-${provider.id}`, async () => {
      try {
        const updated = await api.listProviderModels(provider.id);
        if (await refresh()) setNotice(`${provider.name}: found ${updated.models.length} model${updated.models.length === 1 ? '' : 's'}.`);
      } catch (e) { setError(`${provider.name}: ${errorMessage(e)}`); }
    });
  }
  async function testModel(provider: ProviderConnection, modelId: string) {
    await run(`infer-${provider.id}-${modelId}`, async () => {
      let message = '';
      try { message = (await api.testProviderInference(provider.id, modelId)).message; }
      catch (e) { setError(`${provider.name}: ${errorMessage(e)}`); }
      if (await refresh() && message) setNotice(`${provider.name}: ${message}`);
    });
  }
  async function remove(provider: ProviderConnection) {
    await run(`remove-${provider.id}`, async () => {
      try {
        await api.deleteProvider(provider.id);
        setConfirmRemove(null);
        if (editing !== 'new' && editing?.id === provider.id) setEditing(null);
        if (await refresh()) setNotice(`${provider.name} was removed.`);
      } catch (e) { setError(`${provider.name}: ${errorMessage(e)}`); await refresh(); }
    });
  }
  async function deleteKey(provider: ProviderConnection) {
    await run(`key-${provider.id}`, async () => {
      try {
        await api.deleteProviderApiKey(provider.id);
        setConfirmDeleteKey(null);
        if (editing !== 'new' && editing?.id === provider.id) setEditing(null);
        if (await refresh()) setNotice(`${provider.name}: API key deleted. Provider settings and chat history were kept.`);
      } catch (e) { setError(`${provider.name}: ${errorMessage(e)}`); await refresh(); }
    });
  }
  function use(provider: ProviderConnection, modelId: string) {
    void run(`use-${provider.id}-${modelId}`, () => onSaveSelection({ providerId: provider.id, modelId }));
  }

  function modelList(provider: ProviderConnection, manageable: boolean) {
    if (!provider.models.length) return null;
    const query = modelQueries[provider.id] ?? '';
    const matches = provider.models.filter(model => model.id.toLowerCase().includes(query.trim().toLowerCase()));
    const unconfigured = provider.models.filter(model => !model.contextLength).length;
    return <>
      {provider.models.length > 10 && <label className="provider-model-search">Find a model<input type="search" aria-label={`Find models in ${provider.name}`} value={query} onChange={event => setModelQueries(current => ({ ...current, [provider.id]: event.target.value }))} placeholder="Search model IDs" /></label>}
      <ul className="provider-models" aria-label={`${provider.name} models`}>
        {matches.slice(0, 20).map(model => {
          const ready = remoteModelReady(provider, model);
          const inUse = selection.providerId === provider.id && selection.modelId === model.id;
          const meta = [
            model.contextLength ? `${formatTokens(model.contextLength)} context` : 'context not set',
            model.maxOutputTokens ? `${formatTokens(model.maxOutputTokens)} max output` : '',
            TOOL_LABELS[model.toolSupport],
            model.supportsImages ? 'images' : '',
          ].filter(Boolean).join(' · ');
          return <li key={model.id} className="provider-model">
            <div className="provider-model-info"><strong>{model.id}</strong><small>{meta}</small></div>
            {inUse && <span className={`provider-chip${ready ? ' verified' : ''}`}>{ready ? 'In use' : 'Selected · unavailable'}</span>}
            {!inUse && ready && <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => use(provider, model.id)}>Use</button>}
            {!ready && manageable && !provider.verified &&
              <button type="button" className="secondary" disabled={actionsDisabled || !!pending} title="Send a tiny 1-token inference request to verify this endpoint serves the model" onClick={() => void testModel(provider, model.id)}>{pending === `infer-${provider.id}-${model.id}` ? 'Testing…' : 'Test inference'}</button>}
          </li>;
        })}
      </ul>
      {matches.length > 20 && <p className="provider-hint">Showing 20 of {matches.length} models. Search to find another model.</p>}
      {matches.length === 0 && <p className="provider-hint" role="status">No models match “{query}”.</p>}
      {manageable && provider.verified && unconfigured > 0 &&
        <p className="provider-hint">Set a context capacity under Edit for {unconfigured === 1 ? 'the unconfigured model' : `${unconfigured} unconfigured models`} before {unconfigured === 1 ? 'it' : 'they'} can be selected.</p>}
    </>;
  }
  function card(provider: ProviderConnection, manageable: boolean) {
    const tested = provider.lastTestedAt ? `Last tested ${new Date(provider.lastTestedAt).toLocaleString()}` : 'Not tested yet';
    const credential = provider.hasApiKey ? 'API key saved' : providerRequiresApiKey(provider.apiFormat, provider.baseUrl) ? 'No API key saved' : 'No key required';
    return <section className="provider-card" key={provider.id} aria-label={provider.name}>
      <div className="provider-card-head">
        <span className="provider-icon"><Cloud size={18} /></span>
        <div className="provider-title">
          <strong>{provider.name}</strong>
          <small>{FORMAT_LABELS[provider.apiFormat] ?? provider.apiFormat} · <code>{provider.baseUrl}</code></small>
        </div>
        <span className={`provider-chip${provider.verified ? ' verified' : ''}`}>{provider.verified ? 'Verified' : 'Not verified'}</span>
      </div>
      <p className="provider-meta">{tested} · {credential}</p>
      {modelList(provider, manageable)}
      {!provider.verified && manageable &&
        <p className="provider-hint">Test connection verifies via the model list; if the endpoint has none, add a model ID under Edit and run its tiny Test inference request.</p>}
      {manageable && <div className="provider-actions">
        <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => void testConnection(provider)}>{pending === `test-${provider.id}` ? 'Testing…' : provider.lastTestedAt ? 'Test again' : 'Test connection'}</button>
        <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => void discover(provider)}>{pending === `discover-${provider.id}` ? 'Refreshing…' : 'Refresh models'}</button>
        <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => { setError(''); setNotice(''); setEditing(provider); }}>Edit</button>
        {provider.hasApiKey && (confirmDeleteKey === provider.id
          ? <span className="provider-confirm">Delete the saved API key? This connection will need a new test.
            <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => void deleteKey(provider)}>{pending === `key-${provider.id}` ? 'Deleting…' : 'Delete API key'}</button>
            <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => setConfirmDeleteKey(null)}>Cancel</button>
          </span>
          : <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => { setConfirmRemove(null); setConfirmDeleteKey(provider.id); }}>Delete API key</button>)}
        {confirmRemove === provider.id
          ? <span className="provider-confirm">Remove {provider.name} and its saved key?
            <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => void remove(provider)}>{pending === `remove-${provider.id}` ? 'Removing…' : 'Remove'}</button>
            <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => setConfirmRemove(null)}>Cancel</button>
          </span>
          : <button type="button" className="secondary" disabled={actionsDisabled || !!pending} onClick={() => { setConfirmDeleteKey(null); setConfirmRemove(provider.id); }}>Remove</button>}
      </div>}
    </section>;
  }

  return <div className="providers-tab">
    <div className="section-heading provider-heading">
      <div>
        <h3>API providers</h3>
        <p className="provider-sub">Connect a Chat Completions, Responses or Claude endpoint — hosted or on this computer. Verified models with a context capacity can be selected for chat; selecting one sends requests to its endpoint.</p>
      </div>
      {editing === null && custom.length > 0 && <button type="button" className="secondary" onClick={() => { setError(''); setNotice(''); setEditing('new'); }}><Plus size={14} />Add provider</button>}
    </div>
    {editing !== null && <ProviderEditor
      key={editing === 'new' ? 'new' : editing.id}
      provider={editing === 'new' ? undefined : editing}
      busy={nativeAvailable && (busy || !!pending)}
      onClose={() => setEditing(null)}
      onSaved={message => setNotice(message)}
      onRefresh={onRefresh} />}
    {notice && <p className="success" role="status">{notice}</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {!custom.length && editing === null && <div className="provider-empty">
      <Cloud size={28} />
      <h3>No API providers yet</h3>
      <p>Add an OpenAI-compatible or Claude endpoint — a hosted API or a local engine like llama.cpp — and verified models appear in the chat model picker.</p>
      <button type="button" className="primary" onClick={() => { setError(''); setNotice(''); setEditing('new'); }}><Plus size={14} />Add provider</button>
    </div>}
    {custom.length > 0 && <div className="provider-list">{custom.map(provider => card(provider, true))}</div>}
    {subscriptions.length > 0 && <>
      <h4 className="provider-subheading">Account subscriptions</h4>
      <p className="provider-sub">Signed-in account providers managed outside this page — they are not selectable in this build.</p>
      <div className="provider-list">{subscriptions.map(provider => card(provider, false))}</div>
    </>}
  </div>;
}
