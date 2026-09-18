import { useEffect, useState } from 'react';
import type { ModelSelection, ProviderConnection } from '../lib/types';
import { localModels } from '../lib/localModels';

interface Props {
  providers: ProviderConnection[];
  selection: ModelSelection;
  busy: boolean;
  onSave: (selection: ModelSelection) => Promise<void>;
  localFilename?: string;
  onLocalModelChange?: (filename: string) => void;
  localOptions?: { filename: string; label: string }[];
}

export function ModelSelectorPanel({ providers, selection, busy, onSave, localFilename, onLocalModelChange, localOptions = localModels.map(model => ({ filename: model.filename, label: model.filename })) }: Props) {
  const [draft, setDraft] = useState(selection);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => setDraft(selection), [selection]);
  const provider = providers.find(item => item.id === draft.providerId);
  const invalidProvider = Boolean(draft.providerId && !provider);
  const isSubscription = provider?.apiFormat === 'chatgpt-subscription' || provider?.apiFormat === 'grok-subscription';
  async function save() {
    if (draft.providerId && !draft.modelId.trim()) { setError('Enter a model ID before saving.'); return; }
    setSaving(true); setError('');
    try { await onSave({ ...draft, modelId: draft.providerId ? draft.modelId.trim() : '' }); }
    catch (e) { setError(String(e)); }
    finally { setSaving(false); }
  }
  return <section className="model-selection-panel" aria-labelledby="next-model-heading">
    <div className="section-heading"><div><p className="eyebrow">NEXT MESSAGE</p><h2 id="next-model-heading">Model selection</h2></div><span className="selection-badge">{draft.providerId ? (isSubscription ? 'Subscription (Quota included)' : 'API model') : 'Local model'}</span></div>
    <p className="section-description">Choose exactly where the next message will go. Existing conversations keep their saved choice; new conversations inherit this preference.</p>
    <fieldset disabled={busy || saving}>
      <label>Provider
        <select aria-label="Inference provider" value={draft.providerId ?? ''} onChange={event => { setDraft({ providerId: event.target.value || null, modelId: '' }); setError(''); }}>
          <option value="">Local · llama.cpp</option>
          {providers.map(item => <option key={item.id} value={item.id}>{item.name}{item.verified ? '' : ' · test required'}</option>)}
        </select>
      </label>
      {draft.providerId ? <>
        <label>Model ID
          <input aria-label="Remote model ID" list={provider ? `models-${provider.id}` : undefined} value={draft.modelId} onChange={event => setDraft(current => ({ ...current, modelId: event.target.value }))} placeholder="Enter a model ID manually" />
          {provider && <datalist id={`models-${provider.id}`}>{provider.models.map(model => <option key={model.id} value={model.id} />)}</datalist>}
          <small>Model listing is optional. Manual IDs work when a provider does not expose /v1/models.</small>
        </label>
        {invalidProvider && <p className="error" role="alert">This provider was deleted. Choose another provider; conversation history is preserved.</p>}
        {provider && !provider.verified && <p className="selection-warning" role="status">Test this connection successfully in the Providers section before sending.</p>}
        {provider && provider.verified && draft.modelId && !provider.models.some(model => model.id === draft.modelId) && <p className="selection-warning" role="status">This is a manual model ID. Configure its context capacity in Providers before sending.</p>}
      </> : <><label>Local model<select aria-label="Local model option" value={localFilename ?? localOptions[0]?.filename ?? ''} onChange={event => onLocalModelChange?.(event.target.value)} disabled={!onLocalModelChange}>{localOptions.map(model => <option key={model.filename} value={model.filename}>{model.label}</option>)}</select></label><p className="selection-note">Choose a downloaded model to load it. Add more models in the Models tab below. Local model files and runtime are shared across local conversations.</p></>}
    </fieldset>
    {error && <p className="error" role="alert">{error}</p>}
    <div className="form-footer"><small>{busy ? 'Selection is locked during generation.' : isSubscription ? 'Messages and relevant tool results are sent to your subscription model using included monthly quota.' : 'Messages and relevant tool results are sent to the selected model. Remote usage may incur provider charges.'}</small><button className="primary" type="button" disabled={busy || saving || invalidProvider || (Boolean(draft.providerId) && !draft.modelId.trim())} onClick={() => void save()}>{saving ? 'Saving…' : 'Save selection'}</button></div>
  </section>;
}
