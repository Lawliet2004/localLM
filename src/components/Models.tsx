import { useCallback, useEffect, useRef, useState, type SetStateAction } from 'react';
import { Cpu, Play, Square } from 'lucide-react';
import type { InstalledModel, Preferences, RuntimeConfig, RuntimeStatus } from '../lib/types';
import type { ModelSelection, ProviderConnection } from '../lib/types';
import { RuntimeForm } from './RuntimeForm';
import { HardwareStatus } from './HardwareStatus';
import { RuntimeDiagnostics } from './RuntimeDiagnostics';
import { ModelDownload } from './ModelDownload';
import { RuntimeDownload } from './RuntimeDownload';
import { open } from '@tauri-apps/plugin-dialog';
import { ModelSelectorPanel } from './ModelSelectorPanel';
import { ProviderManager } from './ProviderManager';
import { bonsaiFilename, localModels, modelLabel } from '../lib/localModels';
import { isBonsai2Filename } from '../lib/modelStudio';
import { api, nativeAvailable } from '../lib/api';
import { ModelLibrary } from './ModelLibrary';
import { samePath } from '../lib/pathUtils';

interface Props {
  config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus; busy: boolean;
  providers?: ProviderConnection[]; selection?: ModelSelection;
  onSaveConfig: (config: RuntimeConfig) => Promise<void>;
  onSavePreferences: (preferences: Preferences) => Promise<void>;
  onSaveSelection?: (selection: ModelSelection) => Promise<void>;
  onProvidersChanged?: () => Promise<void>;
  onLoad: () => void; onUnload: () => void;
  requestedModel?: string;
  onRequestedModelHandled?: () => void;
  onRefresh?: () => Promise<void>;
}
export function Models({ config, preferences, runtime, busy, providers = [], selection = { providerId: null, modelId: '' }, onSaveConfig, onSavePreferences, onSaveSelection = async () => {}, onProvidersChanged = async () => {}, onLoad, onUnload, requestedModel, onRequestedModelHandled, onRefresh }: Props) {
  const [draft, setDraft] = useState(preferences);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [localFilename, setLocalFilename] = useState(requestedModel ?? localModels.find(model => preferences.modelPath.endsWith(model.filename))?.filename ?? localModels[0].filename);
  const [tab, setTab] = useState<'library' | 'model' | 'runtime' | 'generation' | 'providers' | 'diagnostics'>('library');
  const [installed, setInstalled] = useState<InstalledModel[]>([]);
  const [libraryLoading, setLibraryLoading] = useState(Boolean(nativeAvailable));
  const [switching, setSwitching] = useState(false);
  const refreshLibrary = useCallback(async () => {
    if (!nativeAvailable) return;
    setLibraryLoading(true);
    try { setInstalled(await api.listInstalledModels()); }
    finally { setLibraryLoading(false); }
  }, []);
  useEffect(() => { void refreshLibrary().catch(e => setError(String(e))); }, [refreshLibrary]);
  async function useModel(model: InstalledModel) {
    setError(''); setNotice('');
    setSwitching(true);
    try {
      await api.useInstalledModel(model.id);
      await onSaveSelection({ providerId: null, modelId: '' });
      if (model.projectorPath) {
        const next = { ...draftRef.current, projectorPath: model.projectorPath };
        updateDraft(next);
        await onSavePreferences(next);
      }
      setLocalFilename(model.id);
    } finally { try { await onRefresh?.(); } finally { setSwitching(false); } }
  }
  const draftRef = useRef(draft);
  const selectionRequest = useRef(0);
  const modelMatches = runtime.phase === 'ready' && samePath(runtime.modelPath, preferences.modelPath);

  useEffect(() => {
    draftRef.current = preferences;
    setDraft(preferences);
    const matched = localModels.find(model => preferences.modelPath.endsWith(model.filename));
    if (matched) {
      setLocalFilename(matched.filename);
    }
  }, [preferences]);

  function updateDraft(value: SetStateAction<Preferences>) {
    const next = typeof value === 'function' ? value(draftRef.current) : value;
    draftRef.current = next;
    setDraft(next);
  }

  async function selectLocalModel(filename: string) {
    const downloaded = installed.find(m => m.id === filename || m.filename === filename);
    if (downloaded) {
      try { await useModel(downloaded); setNotice(`${downloaded.filename} is ready to use.`); }
      catch (e) { setError(String(e)); }
      return;
    }
    const request = ++selectionRequest.current;
    setLocalFilename(filename);
    try {
      const info = await api.modelDownloadInfo(filename);
      if (info.destinationExists) {
        // Read the latest draft after the asynchronous catalog lookup. This
        // prevents a rapid second selection from saving the first model back
        // over the user's newer choice.
        if (request !== selectionRequest.current) return;
        const next = { ...draftRef.current, modelPath: info.destination };
        updateDraft(next);
        await onSavePreferences(next);
        // If another selection started while the IPC write was in flight,
        // restore the newer draft after the older write completes. This keeps
        // slow disk/IPC responses from making the first model win the race.
        if (request !== selectionRequest.current && draftRef.current.modelPath !== next.modelPath) {
          await onSavePreferences(draftRef.current);
          return;
        }
        if (request === selectionRequest.current) {
          setNotice(`Saved ${filename} as active model. ${runtime.phase === 'ready' ? 'Click "Switch to ' + filename + '" or "Load selected model" to apply.' : 'Click "Load model" to start it.'}`);
        }
      } else {
        if (request !== selectionRequest.current) return;
        setTab('model');
        setNotice(`Download or verify ${filename} in Model files below.`);
      }
    } catch {
      setTab('model');
      setNotice('Choose Download or Verify below, then use the verified file.');
    }
  }

  useEffect(() => {
    if (requestedModel && !libraryLoading) {
      void selectLocalModel(requestedModel);
      // This request came from the chat picker and is intentionally one-shot.
      // Leaving it set would re-select the old model every time this page is
      // mounted again.
      onRequestedModelHandled?.();
    }
  }, [requestedModel, libraryLoading]);

  const pendingConfig = runtime.phase === 'ready' && runtime.loadedConfig &&
    (Object.keys(config) as (keyof RuntimeConfig)[]).some(key => config[key] !== runtime.loadedConfig?.[key]);
  busy = busy || switching;
  return <div className="settings-page models-page">
    <div className="page-heading"><p className="eyebrow">LOCAL &amp; REMOTE HARNESS</p><h1>Models &amp; Runtime</h1><p>Run local GGUF models with llama.cpp, tune GPU &amp; CPU runtime settings, or configure remote providers, API keys, proxies, and subscriptions.</p></div>
    <div className="model-status"><div className="model-icon"><Cpu size={24} /></div><div><strong>{runtime.modelPath?.split(/[\\/]/).pop() || 'Your local model'}</strong><p><span className={`status-dot ${runtime.phase === 'ready' ? 'ready' : ''}`} />{runtime.message}</p></div>{runtime.phase === 'ready' ? <>
      {preferences.modelPath && !modelMatches && <button className="primary" disabled={busy} onClick={onLoad}><Play size={14} />Switch to {preferences.modelPath.split(/[\\/]/).pop()}</button>}
      <button className="secondary" disabled={busy} onClick={onUnload}><Square size={14} />Unload</button>
    </> : <button className="primary" disabled={busy || !preferences.modelPath || !preferences.runtimePath} onClick={onLoad}><Play size={14} />{busy ? 'Loading…' : 'Load model'}</button>}</div>
    {tab === 'runtime' && <HardwareStatus runtime={runtime} />}
    {runtime.phase === 'ready' && !modelMatches && <div className="loaded-settings"><p role="status">A different model file is saved. Reload to use {preferences.modelPath.split(/[\\/]/).pop()}.</p><button type="button" className="primary" disabled={busy} onClick={onLoad}>Load selected model</button></div>}
    {pendingConfig && <div className="loaded-settings"><p role="status">Saved configuration is not active. Context: {config.contextLength.toLocaleString()} saved / {runtime.loadedConfig!.contextLength.toLocaleString()} loaded. Apply to reload the model with your saved settings.</p><button className="primary" disabled={busy} onClick={onLoad}>Apply saved configuration</button></div>}
    <div className="tabs" role="tablist" aria-label="Model settings">{(['library','runtime','generation','providers','model','diagnostics'] as const).map(value => <button role="tab" aria-selected={tab === value} key={value} onClick={() => { setTab(value); setNotice(''); }}>{value === 'library' ? 'Models' : value === 'model' ? 'Model files' : value === 'runtime' ? 'Runtime' : value === 'generation' ? 'Generation' : value === 'providers' ? 'API Keys, Proxies & Subscriptions' : 'Diagnostics'}</button>)}</div>
    {notice && <p className="success" role="status">{notice}</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {tab === 'library' ? <ModelLibrary models={installed} loading={libraryLoading} busy={busy || switching} selectedPath={preferences.modelPath} loadedPath={runtime.modelPath} onRefresh={refreshLibrary} onUse={useModel} onDeleted={async () => { await onRefresh?.(); }} /> : tab === 'providers' ? <div className="providers-tab-content"><ModelSelectorPanel providers={providers} selection={selection} busy={busy || switching} onSave={async sel => {
      await onSaveSelection(sel);
      if (!sel.providerId && localFilename) {
        await selectLocalModel(installed.find(m => samePath(m.path, preferences.modelPath))?.id ?? localFilename);
      }
    }} localOptions={nativeAvailable ? installed.map(m => ({ filename: m.id, label: modelLabel(m.filename) })) : undefined} localFilename={installed.find(m => samePath(m.path, preferences.modelPath))?.id ?? localFilename} onLocalModelChange={filename => void selectLocalModel(filename)} /><ProviderManager providers={providers} busy={busy} onChanged={onProvidersChanged} /></div> : <>
    {tab === 'model' && localModels.some(m => m.filename === localFilename) && <ModelDownload key={localFilename} filename={localFilename} busy={busy} onSelect={path => { updateDraft(current => ({ ...current, modelPath: path })); setNotice('Verified model selected. Save settings to apply it, then load the model.'); }} />}
    {draft.modelPath.endsWith(bonsaiFilename) && <p className="selection-warning">Bonsai Q2_0 needs Prism build prism-b9601-68faa14 with its CUDA DLLs. The standard CUDA installer below is for MiniCPM. Bonsai supports up to 65,536 context tokens; use that value for the full model context, or choose a smaller value if memory is limited.</p>}
    {isBonsai2Filename(draft.modelPath) && <p className="selection-warning">Ternary Bonsai 2 needs PrismML llama.cpp prism-b10709 or newer (scripts/prepare-runtime.ps1 -Bonsai2). The standard CUDA installer and the older prism-b9601 Bonsai 8B runtime cannot load PTQ1_0. First load uses 4,096 context, automatic GPU fit, and no vision projector. Attach the mmproj later from Model files if you want image input.</p>}
    {tab === 'model' && <RuntimeDownload busy={busy} onSelect={path => { updateDraft(current => ({ ...current, runtimePath: path })); setNotice('Installed runtime selected. Save settings to apply it.'); }} />}
    {tab === 'diagnostics' ? <RuntimeDiagnostics /> : tab === 'runtime' ? <RuntimeForm key={preferences.modelPath} modelPath={preferences.modelPath} runtime={runtime} initial={config} busy={busy || switching} onSave={async value => { try { setError(''); await onSaveConfig(value); if (runtime.phase === 'ready' || runtime.phase === 'error') { setNotice('Runtime configuration saved. Reloading the model so the new context applies.'); onLoad(); } else { setNotice('Runtime configuration saved. Load the model to use it.'); } } catch (e) { setError(String(e)); } }} /> :
      <form className="runtime-form" onSubmit={async event => { event.preventDefault(); try { setError(''); await onSavePreferences(draft); setNotice('Settings saved.'); } catch (e) { setError(String(e)); } }}>
        <fieldset disabled={busy}>{tab === 'model' ? <><legend>Local files</legend><p className="section-description">Use a GGUF model and a compatible llama.cpp runtime. Q6_K is the recommended starting point for MiniCPM5-2B.</p><label>llama-server executable<input aria-label="llama-server executable" placeholder="C:\path\to\llama-server.exe" value={draft.runtimePath} onChange={e => updateDraft(current => ({ ...current, runtimePath: e.target.value }))} required spellCheck={false} /><small>Choose the CUDA build to use your NVIDIA GPU.</small></label><label>GGUF model file<input aria-label="GGUF model file" placeholder="C:\path\to\MiniCPM5-2B.Q6_K.gguf" value={draft.modelPath} onChange={e => updateDraft(current => ({ ...current, modelPath: e.target.value }))} required spellCheck={false} /></label><label>Vision projector (optional, enables image input)<input aria-label="Vision projector file" placeholder="C:\path\to\mmproj-model-f16.gguf (leave empty for text-only)" value={draft.projectorPath ?? ''} onChange={e => updateDraft(current => ({ ...current, projectorPath: e.target.value }))} spellCheck={false} /><small>Multimodal GGUF models need their matching mmproj file to see images. Leave empty for text-only chat.</small></label></> : <><legend>Response behavior</legend><div className="field-pair"><label>Temperature<input type="number" min="0" max="2" step="0.05" required value={draft.temperature} onChange={e => updateDraft(current => ({ ...current, temperature: e.target.valueAsNumber }))} /></label><label>Top-p<input type="number" min="0.01" max="1" step="0.01" required value={draft.topP} onChange={e => updateDraft(current => ({ ...current, topP: e.target.valueAsNumber }))} /></label></div><label>Maximum response tokens<input type="number" min="1" max="32768" required value={draft.maxTokens} onChange={e => updateDraft(current => ({ ...current, maxTokens: e.target.valueAsNumber }))} /><small>Includes reasoning and the final answer. This reserve must leave room for the prompt in the loaded context. For reasoning models such as ZAYA1, start with 2,048 tokens or more.</small></label>
            {draft.maxTokens >= config.contextLength && <p className="error" role="alert">The response reserve uses the entire {config.contextLength.toLocaleString()}-token context, so every message will fail. Lower this value or increase Context window and reload the model.</p>}<label>System instructions<textarea rows={5} maxLength={32768} value={draft.systemPrompt} onChange={e => updateDraft(current => ({ ...current, systemPrompt: e.target.value }))} /></label></>}</fieldset>
        {tab === 'model' && <div className="file-actions">{(['runtimePath','modelPath','projectorPath'] as const).map(key => <button type="button" className="secondary" key={key} disabled={busy} onClick={async () => {
          try {
            const path = await open({ multiple: false, directory: false, filters: [{ name: key === 'modelPath' || key === 'projectorPath' ? 'GGUF models' : 'Executable', extensions: [key === 'modelPath' || key === 'projectorPath' ? 'gguf' : 'exe'] }] });
            if (path) updateDraft(current => ({ ...current, [key]: path }));
          } catch (e) { setError(String(e)); }
        }}>{key === 'modelPath' ? 'Browse model…' : key === 'projectorPath' ? 'Browse projector…' : 'Browse runtime…'}</button>)}</div>}
        <div className="form-footer"><small>{tab === 'model' ? 'Model files stay on your computer.' : 'Applies to your next response.'}</small><button className="primary" disabled={busy}>Save settings</button></div>
      </form>}
    </>}
  </div>;
}
