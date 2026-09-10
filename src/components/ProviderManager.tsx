import { useEffect, useState } from 'react';
import { Plus, RefreshCw, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { ProviderConnection, ProviderDraft, RemoteModel, ToolSupport } from '../lib/types';

interface Props { providers: ProviderConnection[]; busy: boolean; onChanged: () => Promise<void>; }
const emptyDraft = (): ProviderDraft => ({ name: '', apiFormat: 'openai-chat-completions', baseUrl: 'https://', models: [] });

export function ProviderManager({ providers, busy, onChanged }: Props) {
  const [selectedId, setSelectedId] = useState<string | null>(providers[0]?.id ?? null);
  const [draft, setDraft] = useState<ProviderDraft>(emptyDraft());
  const [apiKey, setApiKey] = useState('');
  const [modelId, setModelId] = useState('');
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [working, setWorking] = useState(false);
  const selected = providers.find(provider => provider.id === selectedId);
  const unsaved = Boolean(selected && (apiKey || draft.name !== selected.name || draft.baseUrl !== selected.baseUrl || JSON.stringify(draft.models) !== JSON.stringify(selected.models)));
  useEffect(() => { setNotice(''); setError(''); }, [selectedId]);
  useEffect(() => {
    if (!selected) { setDraft(emptyDraft()); setApiKey(''); return; }
    setDraft({ id: selected.id, name: selected.name, apiFormat: selected.apiFormat, baseUrl: selected.baseUrl, models: selected.models });
    setApiKey(''); setModelId('');
  }, [selectedId, selected?.lastTestedAt, selected?.verified, providers.length]);
  function updateModel(index: number, patch: Partial<RemoteModel>) {
    setDraft(current => ({ ...current, models: current.models.map((model, modelIndex) => modelIndex === index ? { ...model, ...patch } : model) }));
  }
  async function save() {
    setWorking(true); setNotice(''); setError('');
    try {
      const saved = await api.saveProvider({ ...draft, apiKey: apiKey || undefined });
      setSelectedId(saved.id); setApiKey(''); setNotice('Provider saved. Test the connection before sending messages.'); await onChanged();
    } catch (e) { setError(errorMessage(e)); }
    finally { setWorking(false); }
  }
  async function test() {
    if (!selected) return;
    setWorking(true); setNotice(''); setError('');
    try { const result = await api.testProvider(selected.id); await onChanged(); setNotice(result.message); }
    catch (e) { setError(errorMessage(e)); await onChanged().catch(() => {}); }
    finally { setWorking(false); }
  }
  async function testInference(modelIdToTest?: string) {
    if (!selected || !modelIdToTest) return;
    setWorking(true); setNotice(''); setError('');
    try {
      const result = await api.testProviderInference(selected.id, modelIdToTest);
      await onChanged();
      setNotice(result.message);
    } catch (e) {
      setError(errorMessage(e));
      await onChanged().catch(() => {});
    } finally {
      setWorking(false);
    }
  }
  async function refreshModels() {
    if (!selected) return;
    setWorking(true); setNotice(''); setError('');
    try { const updated = await api.listProviderModels(selected.id); setDraft(current => ({ ...current, models: updated.models })); setNotice(`Loaded ${updated.models.length} model${updated.models.length === 1 ? '' : 's'}. Configure limits below.`); await onChanged(); }
    catch (e) { setError(errorMessage(e)); }
    finally { setWorking(false); }
  }
  async function remove() {
    if (!selected || !window.confirm(`Delete ${selected.name}? Conversation history stays on this device, but affected chats will need another provider.`)) return;
    setWorking(true); setNotice(''); setError('');
    try { await api.deleteProvider(selected.id); setSelectedId(null); setNotice('Provider deleted. Affected conversations need another selection.'); await onChanged(); }
    catch (e) { setError(errorMessage(e)); }
    finally { setWorking(false); }
  }
  return <section className="providers-section" aria-labelledby="providers-heading">
    <div className="section-heading"><div><p className="eyebrow">REMOTE CONNECTIONS</p><h2 id="providers-heading">Providers</h2></div><button className="secondary" type="button" disabled={busy || working} onClick={() => { setSelectedId(null); setDraft(emptyDraft()); setApiKey(''); setNotice(''); setError(''); }}><Plus size={14} />Add provider</button></div>
    <p className="section-description">OpenAI-compatible Chat Completions (DeepSeek, GPT, Ollama, vLLM, Gemini OpenAI endpoint) and native Anthropic Messages. DeepSeek-V3/R1 and other OpenAI-compatible hosts run prefix-cache aware through the shared adapter. API keys are encrypted in the desktop vault and are never shown again.</p>
    {providers.length > 0 && <div className="provider-list" role="list" aria-label="Saved providers">{providers.map(provider => <button type="button" role="listitem" className={selectedId === provider.id ? 'provider-row selected' : 'provider-row'} key={provider.id} disabled={busy || working} onClick={() => setSelectedId(provider.id)}><span><strong>{provider.name}</strong><small>{provider.baseUrl}</small></span><span className={provider.verified ? 'provider-status verified' : 'provider-status'}>{provider.verified ? 'Verified' : 'Test required'}</span></button>)}</div>}
    {!selected && <div className="provider-empty" role="status"><strong>Add a provider connection</strong><p>Use a HTTPS API endpoint, save its key, then run an explicit connection test. The test checks connectivity/model listing only; it does not send a billable inference request.</p></div>}
    {selected && <div className="provider-editor"><div className="provider-editor-header"><div><h3>{selected.name}</h3><p className="provider-status-line">{selected.verified ? 'Verified connection' : 'Not verified'} · {selected.hasApiKey ? 'Encrypted key saved' : 'No key saved'}</p></div><button className="icon-button" aria-label="Delete provider" title="Delete provider" disabled={busy || working} onClick={() => void remove()}><Trash2 size={15} /></button></div>
      <fieldset disabled={busy || working}><label>Connection name<input aria-label="Provider name" value={draft.name} onChange={event => setDraft(current => ({ ...current, name: event.target.value }))} maxLength={120} required /></label><label>API format<select aria-label="API format" value={draft.apiFormat} onChange={event => setDraft(current => ({ ...current, apiFormat: event.target.value as ProviderDraft['apiFormat'] }))}><option value="openai-chat-completions">OpenAI-compatible Chat Completions</option><option value="anthropic-messages">Anthropic Messages</option></select></label><label>Base URL<input aria-label="Provider base URL" type="url" value={draft.baseUrl} onChange={event => setDraft(current => ({ ...current, baseUrl: event.target.value }))} placeholder="https://api.example.com" required /><small>HTTPS is required for remote services. HTTP is allowed only for localhost services. Include the exact API prefix (for example /v1 or /openai). A bare hostname uses /v1. Do not include /chat/completions, credentials or query parameters.</small></label><label>API key<input aria-label="Provider API key" type="password" autoComplete="off" spellCheck={false} value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={selected.hasApiKey ? 'Leave blank to keep the saved key' : 'Enter API key'} /><small>The current key is never returned. Enter a new key to replace it.</small></label></fieldset>
      {unsaved && <p role="status">Save changes before testing the connection or refreshing models.</p>}
      <div className="provider-actions"><button className="primary" type="button" disabled={busy || working} onClick={() => void save()}>Save provider</button><button className="secondary" type="button" disabled={busy || working || unsaved || !selected.hasApiKey} onClick={() => void test()}>Test connection</button><button className="secondary" type="button" disabled={busy || working || unsaved || !selected.hasApiKey || !draft.models.length} onClick={() => void testInference(draft.models[0]?.id)}>Test inference</button><button className="secondary" type="button" disabled={busy || working || unsaved || !selected.hasApiKey} onClick={() => void refreshModels()}><RefreshCw size={13} />Refresh models</button></div>
      <div className="remote-models"><div className="section-heading"><div><h3>Remote models</h3><p className="section-description">Manual IDs are always allowed. Context capacity is required for chat; output capacity and tool support are optional provider-specific settings.</p></div></div>{draft.models.map((model, index) => <div className="remote-model-row" key={`${model.id}-${index}`}><div className="remote-model-id"><label>Model ID<input aria-label={`Model ID ${index + 1}`} value={model.id} onChange={event => updateModel(index, { id: event.target.value })} /></label><button className="secondary" type="button" disabled={busy || working} onClick={() => setDraft(current => ({ ...current, models: current.models.filter((_, modelIndex) => modelIndex !== index) }))}>Remove</button></div><div className="field-pair"><label>Context capacity<input aria-label={`Context capacity ${index + 1}`} type="number" min={128} max={2000000} placeholder="Required" value={model.contextLength ?? ''} onChange={event => updateModel(index, { contextLength: event.target.value ? event.target.valueAsNumber : null })} /></label><label>Max output tokens<input aria-label={`Max output tokens ${index + 1}`} type="number" min={1} max={1000000} placeholder="Provider default" value={model.maxOutputTokens ?? ''} onChange={event => updateModel(index, { maxOutputTokens: event.target.value ? event.target.valueAsNumber : null })} /></label></div><label>Tool calling<select aria-label={`Tool calling ${index + 1}`} value={model.toolSupport} onChange={event => updateModel(index, { toolSupport: event.target.value as ToolSupport })}><option value="unknown">Unknown · try selected tools</option><option value="supported">Supported</option><option value="unsupported">Not supported</option></select></label></div>)}<div className="remote-model-add"><input aria-label="Manual model ID" placeholder="Manual model ID" value={modelId} onChange={event => setModelId(event.target.value)} /><button className="secondary" type="button" disabled={busy || working || !modelId.trim()} onClick={() => { setDraft(current => ({ ...current, models: [...current.models, { id: modelId.trim(), contextLength: null, maxOutputTokens: null, toolSupport: 'unknown' }] })); setModelId(''); }}>Add model</button></div></div>
    </div>}
    {notice && <p className="success" role="status">{notice}</p>}{error && <p className="error" role="alert">{error}</p>}
  </section>;
}
