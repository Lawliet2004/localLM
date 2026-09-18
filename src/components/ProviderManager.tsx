import { useEffect, useState } from 'react';
import { Plus, RefreshCw, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { ProviderConnection, ProviderDraft, RemoteModel, SubscriptionStatus, CliDetectionResult, ToolSupport } from '../lib/types';

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

  // Subscriptions state
  const [chatgptStatus, setChatgptStatus] = useState<SubscriptionStatus | null>(null);
  const [grokStatus, setGrokStatus] = useState<SubscriptionStatus | null>(null);
  const [cliChatgpt, setCliChatgpt] = useState<CliDetectionResult | null>(null);
  const [cliGrok, setCliGrok] = useState<CliDetectionResult | null>(null);
  const [manualProvider, setManualProvider] = useState<'chatgpt' | 'grok' | null>(null);
  const [manualToken, setManualToken] = useState('');
  const [manualAccountId, setManualAccountId] = useState('');
  const [signingIn, setSigningIn] = useState<'chatgpt' | 'grok' | null>(null);

  const selected = providers.find(provider => provider.id === selectedId);
  const isSubscription = draft.apiFormat === 'chatgpt-subscription' || draft.apiFormat === 'grok-subscription';
  const isLoopback = draft.baseUrl.includes('localhost') || draft.baseUrl.includes('127.0.0.1') || draft.baseUrl.includes('::1');
  const hasAuth = Boolean(selected?.hasApiKey || isLoopback || isSubscription);
  const unsaved = Boolean(selected && (apiKey || draft.name !== selected.name || draft.baseUrl !== selected.baseUrl || draft.apiFormat !== selected.apiFormat || JSON.stringify(draft.models) !== JSON.stringify(selected.models)));

  const loadSubscriptions = async () => {
    try {
      const [cgStatus, grStatus, cgCli, grCli] = await Promise.all([
        api.getSubscriptionStatus ? api.getSubscriptionStatus('chatgpt').catch(() => null) : Promise.resolve(null),
        api.getSubscriptionStatus ? api.getSubscriptionStatus('grok').catch(() => null) : Promise.resolve(null),
        api.detectSubscriptionCli ? api.detectSubscriptionCli('chatgpt').catch(() => null) : Promise.resolve(null),
        api.detectSubscriptionCli ? api.detectSubscriptionCli('grok').catch(() => null) : Promise.resolve(null),
      ]);
      if (cgStatus) setChatgptStatus(cgStatus);
      if (grStatus) setGrokStatus(grStatus);
      if (cgCli) setCliChatgpt(cgCli);
      if (grCli) setCliGrok(grCli);
    } catch {
      // Ignore background load error
    }
  };

  useEffect(() => {
    void loadSubscriptions();
  }, [providers.length]);

  useEffect(() => { setNotice(''); setError(''); }, [selectedId]);
  useEffect(() => {
    if (!selected) { setDraft(emptyDraft()); setApiKey(''); return; }
    setDraft({ id: selected.id, name: selected.name, apiFormat: selected.apiFormat, baseUrl: selected.baseUrl, models: selected.models });
    setApiKey(''); setModelId('');
  }, [selectedId, selected?.lastTestedAt, selected?.verified, providers.length]);

  function updateModel(index: number, patch: Partial<RemoteModel>) {
    setDraft(current => ({ ...current, models: current.models.map((model, modelIndex) => modelIndex === index ? { ...model, ...patch } : model) }));
  }

  async function syncCli(provider: 'chatgpt' | 'grok') {
    setWorking(true); setNotice(''); setError('');
    try {
      const status = await api.importSubscriptionCli(provider);
      if (provider === 'chatgpt') setChatgptStatus(status); else setGrokStatus(status);
      setNotice(`Imported ${provider === 'chatgpt' ? 'ChatGPT' : 'Grok'} credentials from CLI successfully.`);
      await onChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWorking(false);
    }
  }

  async function startSignIn(provider: 'chatgpt' | 'grok') {
    setWorking(true); setSigningIn(provider); setNotice(''); setError('');
    try {
      const status = await api.startSubscriptionSignIn(provider);
      if (provider === 'chatgpt') setChatgptStatus(status); else setGrokStatus(status);
      setNotice(`Signed in to ${provider === 'chatgpt' ? 'ChatGPT' : 'Grok'} successfully.`);
      await onChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWorking(false);
      setSigningIn(null);
    }
  }

  async function cancelSignIn() {
    try {
      await api.cancelSubscriptionSignIn();
    } catch {
      // Ignore
    } finally {
      setSigningIn(null);
      setWorking(false);
    }
  }

  async function saveManualToken(provider: 'chatgpt' | 'grok') {
    if (!manualToken.trim()) return;
    setWorking(true); setNotice(''); setError('');
    try {
      const status = await api.saveManualSubscriptionToken(
        provider,
        manualToken.trim(),
        null,
        manualAccountId.trim() || null
      );
      if (provider === 'chatgpt') setChatgptStatus(status); else setGrokStatus(status);
      setNotice(`Saved ${provider === 'chatgpt' ? 'ChatGPT' : 'Grok'} token successfully.`);
      setManualProvider(null);
      setManualToken('');
      setManualAccountId('');
      await onChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWorking(false);
    }
  }

  async function disconnectSub(provider: 'chatgpt' | 'grok') {
    if (!window.confirm(`Disconnect ${provider === 'chatgpt' ? 'ChatGPT' : 'Grok'} subscription?`)) return;
    setWorking(true); setNotice(''); setError('');
    try {
      await api.disconnectSubscription(provider);
      if (api.getSubscriptionStatus) {
        const status = await api.getSubscriptionStatus(provider);
        if (provider === 'chatgpt') setChatgptStatus(status); else setGrokStatus(status);
      }
      setNotice(`Disconnected ${provider === 'chatgpt' ? 'ChatGPT' : 'Grok'} subscription.`);
      await onChanged();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWorking(false);
    }
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
    {/* Subscriptions Section */}
    <div className="subscriptions-section">
      <div className="section-heading">
        <div>
          <p className="eyebrow">SUBSCRIPTION ACCOUNTS</p>
          <h3>ChatGPT &amp; Grok Subscriptions</h3>
        </div>
      </div>
      <p className="section-description">
        Use your existing ChatGPT Plus/Pro/Team or SuperGrok monthly subscriptions directly in LocalLM without per-token API billing. Credentials and refresh tokens are encrypted in your local desktop vault.
      </p>
      <div className="subscriptions-grid">
        {/* ChatGPT Card */}
        <div className="subscription-card">
          <div className="subscription-header">
            <strong>ChatGPT (Plus / Pro / Team)</strong>
            <span className={`provider-status ${chatgptStatus?.connected ? 'verified' : ''}`}>
              {chatgptStatus?.connected ? 'Active' : 'Disconnected'}
            </span>
          </div>
          <div className="subscription-details">
            {chatgptStatus?.connected ? (
              <>
                <span>Account: {chatgptStatus.accountEmail || 'Authenticated Session'}</span>
                <span>Plan: {chatgptStatus.planType || 'ChatGPT Subscription'}</span>
                {chatgptStatus.expiresAt ? (
                  <span>Expires: {new Date(chatgptStatus.expiresAt * 1000).toLocaleDateString()}</span>
                ) : null}
              </>
            ) : (
              <span>Sign in with browser OAuth, sync from Codex CLI, or enter a developer token.</span>
            )}
            {cliChatgpt?.found && !chatgptStatus?.connected && (
              <span className="subscription-cli-badge">
                Found CLI credentials: {cliChatgpt.cliPath}
              </span>
            )}
          </div>
          <div className="subscription-actions">
            {chatgptStatus?.connected ? (
              <>
                <button className="secondary" type="button" disabled={busy || working} onClick={() => void disconnectSub('chatgpt')}>
                  Disconnect
                </button>
                {cliChatgpt?.found && (
                  <button className="secondary" type="button" disabled={busy || working} onClick={() => void syncCli('chatgpt')}>
                    Re-sync from CLI
                  </button>
                )}
              </>
            ) : (
              <>
                {cliChatgpt?.found && (
                  <button className="primary" type="button" disabled={busy || working} onClick={() => void syncCli('chatgpt')}>
                    Sync from CLI
                  </button>
                )}
                <button className="secondary" type="button" disabled={busy || working} onClick={() => void startSignIn('chatgpt')}>
                  {signingIn === 'chatgpt' ? 'Waiting for browser…' : 'Sign in (OAuth)'}
                </button>
                <button className="secondary" type="button" disabled={busy || working} onClick={() => setManualProvider(curr => curr === 'chatgpt' ? null : 'chatgpt')}>
                  Manual token
                </button>
                {signingIn === 'chatgpt' && (
                  <button className="secondary" type="button" onClick={() => void cancelSignIn()}>Cancel</button>
                )}
              </>
            )}
          </div>
          {manualProvider === 'chatgpt' && (
            <div className="subscription-manual-form">
              <input
                aria-label="ChatGPT Access Token"
                type="password"
                placeholder="Access Token (Bearer eyJ...)"
                value={manualToken}
                onChange={e => setManualToken(e.target.value)}
              />
              <input
                aria-label="ChatGPT Account ID (optional)"
                placeholder="Account ID (optional)"
                value={manualAccountId}
                onChange={e => setManualAccountId(e.target.value)}
              />
              <div style={{ display: 'flex', gap: '6px' }}>
                <button className="primary" type="button" disabled={busy || working || !manualToken.trim()} onClick={() => void saveManualToken('chatgpt')}>
                  Save
                </button>
                <button className="secondary" type="button" onClick={() => setManualProvider(null)}>
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>

        {/* Grok Card */}
        <div className="subscription-card">
          <div className="subscription-header">
            <strong>Grok (SuperGrok / Premium+)</strong>
            <span className={`provider-status ${grokStatus?.connected ? 'verified' : ''}`}>
              {grokStatus?.connected ? 'Active' : 'Disconnected'}
            </span>
          </div>
          <div className="subscription-details">
            {grokStatus?.connected ? (
              <>
                <span>Account: {grokStatus.accountEmail || 'Authenticated Session'}</span>
                <span>Plan: {grokStatus.planType || 'SuperGrok Subscription'}</span>
                {grokStatus.expiresAt ? (
                  <span>Expires: {new Date(grokStatus.expiresAt * 1000).toLocaleDateString()}</span>
                ) : null}
              </>
            ) : (
              <span>Sign in with browser OAuth, sync from Grok CLI, or enter a developer token.</span>
            )}
            {cliGrok?.found && !grokStatus?.connected && (
              <span className="subscription-cli-badge">
                Found CLI credentials: {cliGrok.cliPath}
              </span>
            )}
          </div>
          <div className="subscription-actions">
            {grokStatus?.connected ? (
              <>
                <button className="secondary" type="button" disabled={busy || working} onClick={() => void disconnectSub('grok')}>
                  Disconnect
                </button>
                {cliGrok?.found && (
                  <button className="secondary" type="button" disabled={busy || working} onClick={() => void syncCli('grok')}>
                    Re-sync from CLI
                  </button>
                )}
              </>
            ) : (
              <>
                {cliGrok?.found && (
                  <button className="primary" type="button" disabled={busy || working} onClick={() => void syncCli('grok')}>
                    Sync from CLI
                  </button>
                )}
                <button className="secondary" type="button" disabled={busy || working} onClick={() => void startSignIn('grok')}>
                  {signingIn === 'grok' ? 'Waiting for browser…' : 'Sign in (OAuth)'}
                </button>
                <button className="secondary" type="button" disabled={busy || working} onClick={() => setManualProvider(curr => curr === 'grok' ? null : 'grok')}>
                  Manual token
                </button>
                {signingIn === 'grok' && (
                  <button className="secondary" type="button" onClick={() => void cancelSignIn()}>Cancel</button>
                )}
              </>
            )}
          </div>
          {manualProvider === 'grok' && (
            <div className="subscription-manual-form">
              <input
                aria-label="Grok Access Token"
                type="password"
                placeholder="Access Token (Bearer sso... / eyJ...)"
                value={manualToken}
                onChange={e => setManualToken(e.target.value)}
              />
              <div style={{ display: 'flex', gap: '6px' }}>
                <button className="primary" type="button" disabled={busy || working || !manualToken.trim()} onClick={() => void saveManualToken('grok')}>
                  Save
                </button>
                <button className="secondary" type="button" onClick={() => setManualProvider(null)}>
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>

    {/* Provider Connections Section */}
    <div className="section-heading"><div><p className="eyebrow">REMOTE CONNECTIONS &amp; PROXIES</p><h2 id="providers-heading">Providers &amp; Proxies</h2></div><button className="secondary" type="button" disabled={busy || working} onClick={() => { setSelectedId(null); setDraft(emptyDraft()); setApiKey(''); setNotice(''); setError(''); }}><Plus size={14} />Add provider</button></div>
    <p className="section-description">LocalLM supports OpenAI-compatible endpoints, custom proxies and relays (e.g. LiteLLM, Ollama, vLLM, OneAPI), as well as official ChatGPT and Grok subscription connections. API keys, proxy tokens, and subscription credentials are encrypted in your desktop vault and are never displayed in plaintext.</p>
    {providers.length > 0 && <div className="provider-list" role="list" aria-label="Saved providers">{providers.map(provider => <button type="button" role="listitem" className={selectedId === provider.id ? 'provider-row selected' : 'provider-row'} key={provider.id} disabled={busy || working} onClick={() => setSelectedId(provider.id)}><span><strong>{provider.name}</strong><small>{provider.baseUrl}</small></span><span className={provider.verified ? 'provider-status verified' : 'provider-status'}>{provider.verified ? 'Verified' : 'Test required'}</span></button>)}</div>}
    {!selected && <div className="provider-empty" role="status"><strong>Add a provider connection</strong><p>Use an HTTPS API endpoint or local proxy URL, save its key if required, then run an explicit connection test. The test checks connectivity/model listing only; it does not send a billable inference request.</p></div>}
    {selected && <div className="provider-editor"><div className="provider-editor-header"><div><h3>{selected.name}</h3><p className="provider-status-line">{selected.verified ? 'Verified connection' : 'Not verified'} · {selected.hasApiKey ? 'Encrypted credentials saved' : (isLoopback ? 'Loopback engine (no key required)' : 'No credentials saved')}</p></div><button className="icon-button" aria-label="Delete provider" title="Delete provider" disabled={busy || working} onClick={() => void remove()}><Trash2 size={15} /></button></div>
      <fieldset disabled={busy || working}>
        <label>Connection name<input aria-label="Provider name" value={draft.name} onChange={event => setDraft(current => ({ ...current, name: event.target.value }))} maxLength={120} required /></label>
        <label>API format
          <select aria-label="API format" value={draft.apiFormat} onChange={event => setDraft(current => ({ ...current, apiFormat: event.target.value as ProviderDraft['apiFormat'] }))}>
            <option value="openai-chat-completions">OpenAI-compatible Chat Completions / Proxy</option>
            <option value="freetoken-openai">FreeToken loopback engine (OpenAI-compatible, no key)</option>
            <option value="chatgpt-subscription">ChatGPT Subscription (Codex / OAuth)</option>
            <option value="grok-subscription">Grok Subscription (SuperGrok / Build CLI)</option>
          </select>
        </label>
        <label>Base URL<input aria-label="Provider base URL" type="url" value={draft.baseUrl} onChange={event => setDraft(current => ({ ...current, baseUrl: event.target.value }))} placeholder="https://api.openai.com/v1 or http://localhost:8000/v1 (proxy)" required /><small>HTTPS is required for remote services. HTTP is allowed for localhost services (e.g. http://localhost:8000/v1 for local proxies/relays). Include the exact API prefix (for example /v1 or /openai). A bare hostname uses /v1. Do not include /chat/completions, credentials or query parameters.</small></label>
        {isSubscription ? (
          <p className="selection-note">Authentication and automatic token refresh are managed via the Subscription Accounts section above.</p>
        ) : (
          <label>API key<input aria-label="Provider API key" type="password" autoComplete="off" spellCheck={false} value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder={selected.hasApiKey ? 'Leave blank to keep the saved key' : (isLoopback ? 'Optional for loopback engines' : 'Enter API key')} /><small>The current key is never returned. Enter a new key to replace it.</small></label>
        )}
      </fieldset>
      {unsaved && <p role="status">Save changes before testing the connection or refreshing models.</p>}
      <div className="provider-actions">
        <button className="primary" type="button" disabled={busy || working} onClick={() => void save()}>Save provider</button>
        <button className="secondary" type="button" disabled={busy || working || unsaved || !hasAuth} onClick={() => void test()}>Test connection</button>
        <button className="secondary" type="button" disabled={busy || working || unsaved || !hasAuth || !draft.models.length} onClick={() => void testInference(draft.models[0]?.id)}>Test inference</button>
        <button className="secondary" type="button" disabled={busy || working || unsaved || !hasAuth} onClick={() => void refreshModels()}><RefreshCw size={13} />Refresh models</button>
      </div>
      <div className="remote-models"><div className="section-heading"><div><h3>Remote models</h3><p className="section-description">Manual IDs are always allowed. Context capacity is required for chat; output capacity and tool support are optional provider-specific settings.</p></div></div>{draft.models.map((model, index) => <div className="remote-model-row" key={`${model.id}-${index}`}><div className="remote-model-id"><label>Model ID<input aria-label={`Model ID ${index + 1}`} value={model.id} onChange={event => updateModel(index, { id: event.target.value })} /></label><button className="secondary" type="button" disabled={busy || working} onClick={() => setDraft(current => ({ ...current, models: current.models.filter((_, modelIndex) => modelIndex !== index) }))}>Remove</button></div><div className="field-pair"><label>Context capacity<input aria-label={`Context capacity ${index + 1}`} type="number" min={128} max={2000000} placeholder="Required" value={model.contextLength ?? ''} onChange={event => updateModel(index, { contextLength: event.target.value ? event.target.valueAsNumber : null })} /></label><label>Max output tokens<input aria-label={`Max output tokens ${index + 1}`} type="number" min={1} max={1000000} placeholder="Provider default" value={model.maxOutputTokens ?? ''} onChange={event => updateModel(index, { maxOutputTokens: event.target.value ? event.target.valueAsNumber : null })} /></label></div><label><input type="checkbox" checked={model.supportsImages ?? false} onChange={event => updateModel(index, { supportsImages: event.target.checked })} /> Supports image input</label><label>Tool calling<select aria-label={`Tool calling ${index + 1}`} value={model.toolSupport} onChange={event => updateModel(index, { toolSupport: event.target.value as ToolSupport })}><option value="unknown">Unknown · try selected tools</option><option value="supported">Supported</option><option value="unsupported">Not supported</option></select></label></div>)}<div className="remote-model-add"><input aria-label="Manual model ID" placeholder="Manual model ID" value={modelId} onChange={event => setModelId(event.target.value)} /><button className="secondary" type="button" disabled={busy || working || !modelId.trim()} onClick={() => { setDraft(current => ({ ...current, models: [...current.models, { id: modelId.trim(), contextLength: null, maxOutputTokens: null, toolSupport: 'unknown' }] })); setModelId(''); }}>Add model</button></div></div>
    </div>}
    {notice && <p className="success" role="status">{notice}</p>}{error && <p className="error" role="alert">{error}</p>}
  </section>;
}
