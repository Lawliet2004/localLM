import { ManagedStorage } from './ManagedStorage';
import { PresentationDefaults } from './PresentationDefaults';
import { useCallback, useEffect, useRef, useState, type SetStateAction } from 'react';
import { Cpu, Play, Square } from 'lucide-react';
import type { InstalledModel, Preferences, ProviderConnection, RuntimeConfig, RuntimeStatus } from '../lib/types';
import type { ModelSelection } from '../lib/types';
import { RuntimeForm } from './RuntimeForm';
import { HardwareStatus } from './HardwareStatus';
import { RuntimeDiagnostics } from './RuntimeDiagnostics';
import { ModelDownload } from './ModelDownload';
import { RuntimeDownload } from './RuntimeDownload';
import { open } from '@tauri-apps/plugin-dialog';
import { localModels } from '../lib/localModels';
import { api, nativeAvailable } from '../lib/api';
import { ModelLibrary } from './ModelLibrary';
import { Providers } from './Providers';
import { samePath } from '../lib/pathUtils';

interface Props {
  initialTab?: 'library' | 'generation' | 'diagnostics';
  config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus; busy: boolean;
  onSaveConfig: (config: RuntimeConfig) => Promise<void>;
  onSavePreferences: (preferences: Preferences) => Promise<void>;
  onSaveSelection?: (selection: ModelSelection) => Promise<void>;
  onLoad: () => void; onUnload: () => void;
  providers?: ProviderConnection[];
  selection?: ModelSelection;
  requestedModel?: string;
  onRequestedModelHandled?: () => void;
  onRefresh?: () => Promise<void>;
}
export function Models({ config, preferences, runtime, busy, onSaveConfig, onSavePreferences, onSaveSelection = async () => {}, onLoad, onUnload, providers = [], selection = { providerId: null, modelId: '' }, requestedModel, onRequestedModelHandled, onRefresh, initialTab = 'library' }: Props) {
  const [draft, setDraft] = useState(preferences);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [localFilename, setLocalFilename] = useState(requestedModel ?? localModels.find(model => preferences.modelPath.endsWith(model.filename))?.filename ?? localModels[0].filename);
  const [tab, setTab] = useState<'library' | 'providers' | 'model' | 'runtime' | 'generation' | 'diagnostics'>(initialTab);
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

  async function chooseRuntimeProjector() {
    try {
      setError('');
      const path = await open({ multiple: false, directory: false, filters: [{ name: 'Vision projectors', extensions: ['gguf'] }] });
      if (!path) return;
      const next = { ...preferences, projectorPath: path };
      await onSavePreferences(next);
      updateDraft(next);
      setNotice('Projector file selected. Save configuration to apply its loading and placement settings.');
    } catch (error) { setError(String(error)); }
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
    <div className="page-heading"><h1>{initialTab === 'generation' ? 'Inference' : initialTab === 'diagnostics' ? 'Diagnostics' : 'Models & Runtime'}</h1><p>{initialTab === 'diagnostics' ? 'Measure response latency and inspect runtime output and tool support.' : initialTab === 'generation' ? 'Choose reasoning behavior, sampling and response length for future replies.' : 'Run local GGUF models with llama.cpp, connect API providers, and tune GPU & CPU runtime settings.'}</p></div>
    {tab !== 'providers' && <>
    {busy && nativeAvailable && runtime.phase !== 'ready' && <button type="button" className="secondary" onClick={() => { void api.cancelModelLoad().catch(e => setError(String(e))); }}>Cancel model loading</button>}
    <div className="model-status"><div className="model-icon"><Cpu size={24} /></div><div><strong>{runtime.modelPath?.split(/[\\/]/).pop() || 'Your local model'}</strong><p><span className={`status-dot ${runtime.phase === 'ready' ? 'ready' : ''}`} />{runtime.message}</p></div>{runtime.phase === 'ready' && preferences.modelPath && !modelMatches && <button className="primary" disabled={busy} onClick={onLoad}><Play size={14} />Switch to {preferences.modelPath.split(/[\\/]/).pop()}</button>}<button className={runtime.phase === 'ready' ? 'secondary' : 'primary'} disabled={busy || (runtime.phase !== 'ready' && (!preferences.modelPath || !preferences.runtimePath))} onClick={runtime.phase === 'ready' ? onUnload : onLoad}>{runtime.phase === 'ready' ? <Square size={14} /> : <Play size={14} />}{runtime.phase === 'ready' ? 'Unload' : busy && nativeAvailable ? 'Loading…' : 'Load model'}</button></div>
    {tab === 'runtime' && <HardwareStatus runtime={runtime} />}
    {runtime.phase === 'ready' && !modelMatches && <div className="loaded-settings"><p role="status">A different model file is saved. Reload to use {preferences.modelPath.split(/[\\/]/).pop()}.</p><button type="button" className="primary" disabled={busy} onClick={onLoad}>Load selected model</button></div>}
    {pendingConfig && <div className="loaded-settings"><p role="status">Saved configuration is not active. Context: {config.contextLength.toLocaleString()} saved / {runtime.loadedConfig!.contextLength.toLocaleString()} loaded. Apply to reload the model with your saved settings.</p><button className="primary" disabled={busy} onClick={onLoad}>Apply saved configuration</button></div>}
    </>}
    {!['generation', 'diagnostics'].includes(initialTab) && <div className="tabs" role="tablist" aria-label="Model settings">{(['library','providers','runtime','model'] as const).map(value => <button role="tab" aria-selected={tab === value} key={value} onClick={() => { setTab(value); setNotice(''); }}>{value === 'library' ? 'Models' : value === 'providers' ? 'API providers' : value === 'model' ? 'Model files' : value === 'runtime' ? 'Runtime' : 'Model files'}</button>)}</div>}
    {notice && <p className="success" role="status">{notice}</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {tab === 'library' ? <ModelLibrary models={installed} loading={libraryLoading} busy={busy || switching} selectedPath={preferences.modelPath} loadedPath={runtime.modelPath} onRefresh={refreshLibrary} onUse={useModel} onDeleted={async () => { await onRefresh?.(); }} /> : tab === 'providers' ? <Providers providers={providers} selection={selection} busy={busy || switching} onSaveSelection={onSaveSelection} onRefresh={onRefresh} /> : <>
    {tab === 'model' && localModels.some(m => m.filename === localFilename) && <ModelDownload key={localFilename} filename={localFilename} busy={busy} onSelect={path => { updateDraft(current => ({ ...current, modelPath: path })); setNotice('Verified model selected. Save settings to apply it, then load the model.'); }} />}
    {tab === 'model' && <ManagedStorage busy={busy} onMoved={async () => { await refreshLibrary(); await onRefresh?.(); }} />}
    {tab === 'model' && <RuntimeDownload busy={busy} onSelect={path => { updateDraft(current => ({ ...current, runtimePath: path })); setNotice('Installed runtime selected. Save settings to apply it.'); }} />}
    {tab === 'diagnostics' ? <RuntimeDiagnostics /> : tab === 'runtime' ? <RuntimeForm key={preferences.modelPath} modelPath={preferences.modelPath} projectorPath={preferences.projectorPath} onChooseProjector={() => void chooseRuntimeProjector()} runtime={runtime} initial={config} busy={busy || switching} onSave={async value => { try { setError(''); await onSaveConfig(value); if (runtime.phase === 'ready' || runtime.phase === 'error') { setNotice('Runtime configuration saved. Reloading the model to apply context, memory, and projector settings.'); onLoad(); } else { setNotice('Runtime configuration saved. Load the model to use it.'); } } catch (e) { setError(String(e)); } }} /> :
      <form className="runtime-form" onSubmit={async event => { event.preventDefault(); try { setError(''); await onSavePreferences(draft); setNotice('Settings saved.'); } catch (e) { setError(String(e)); } }}>
        <fieldset disabled={busy}>{tab === 'model' ? <><legend>Local files</legend><p className="section-description">Use a GGUF model and a compatible llama.cpp runtime. Q6_K is the recommended starting point for MiniCPM5-2B.</p><div className="preference-group"><div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor="pref-runtime-path">llama-server executable</label><p>Choose the CUDA build to use your NVIDIA GPU.</p></div><input id="pref-runtime-path" aria-label="llama-server executable" placeholder="C:\path\to\llama-server.exe" value={draft.runtimePath} onChange={e => updateDraft(current => ({ ...current, runtimePath: e.target.value }))} required spellCheck={false} /></div><div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor="pref-model-path">GGUF model file</label></div><input id="pref-model-path" aria-label="GGUF model file" placeholder="C:\path\to\MiniCPM5-2B.Q6_K.gguf" value={draft.modelPath} onChange={e => updateDraft(current => ({ ...current, modelPath: e.target.value }))} required spellCheck={false} /></div><div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor="pref-projector-path">Vision projector (required for image input)</label><p>Choose the matching mmproj file from the same model repository, or leave empty for text-only chat.</p></div><input id="pref-projector-path" aria-label="Vision projector file" placeholder="C:\path\to\mmproj-model-f16.gguf (leave empty for text-only)" value={draft.projectorPath ?? ''} onChange={e => updateDraft(current => ({ ...current, projectorPath: e.target.value }))} spellCheck={false} /></div><div className="preference-row preference-row-block"><div className="field-row">{(['runtimePath','modelPath','projectorPath'] as const).map(key => <button type="button" className="secondary" key={key} disabled={busy} onClick={async () => {
          try {
            const path = await open({ multiple: false, directory: false, filters: key === 'runtimePath' ? undefined : [{ name: 'GGUF models', extensions: ['gguf'] }] });
            if (path) updateDraft(current => ({ ...current, [key]: path }));
          } catch (e) { setError(String(e)); }
        }}>{key === 'modelPath' ? 'Browse model…' : key === 'projectorPath' ? 'Browse projector…' : 'Browse runtime…'}</button>)}</div></div></div></> : <><legend>Response behavior</legend><div className="preference-group"><div className="preference-row"><div className="preference-copy"><label htmlFor="pref-reasoning">Reasoning</label><p>Auto favors a direct reply when no tools are offered; otherwise it follows the model default. Quick requests less reasoning where supported. Think requests more effort on models with a verified control. Unknown models keep their default. Answer length stays separate.</p></div><select id="pref-reasoning" value={draft.reasoningMode ?? 'auto'} onChange={e => updateDraft(current => ({ ...current, reasoningMode: e.target.value as 'auto' | 'quick' | 'think' }))}><option value="auto">Auto</option><option value="quick">Quick</option><option value="think">Think</option></select></div><div className="preference-row preference-row-block"><div className="field-pair"><label>Temperature<input type="number" min="0" max="2" step="0.05" required value={draft.temperature} onChange={e => updateDraft(current => ({ ...current, temperature: e.target.valueAsNumber }))} /></label><label>Top-p<input type="number" min="0.01" max="1" step="0.01" required value={draft.topP} onChange={e => updateDraft(current => ({ ...current, topP: e.target.valueAsNumber }))} /></label></div></div><div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor="pref-max-tokens">Maximum response tokens</label><p>Includes reasoning and the final answer. Leave room for the prompt in the loaded context — reasoning models need 2,048 tokens or more.</p></div><input id="pref-max-tokens" type="number" min="1" max="32768" required value={draft.maxTokens} onChange={e => updateDraft(current => ({ ...current, maxTokens: e.target.valueAsNumber }))} />{draft.maxTokens >= config.contextLength && <p className="error" role="alert">The response reserve uses the entire {config.contextLength.toLocaleString()}-token context, so every message will fail. Lower this value or increase Context window and reload the model.</p>}</div><PresentationDefaults /><div className="preference-row preference-row-block"><div className="preference-copy"><label htmlFor="pref-system-prompt">System instructions</label></div><textarea id="pref-system-prompt" aria-label="System instructions" rows={5} maxLength={32768} value={draft.systemPrompt} onChange={e => updateDraft(current => ({ ...current, systemPrompt: e.target.value }))} /></div></div></>}</fieldset>
        <div className="form-footer"><small>{tab === 'model' ? 'Model files stay on your computer.' : 'Applies to your next response.'}</small><button className="primary" disabled={busy}>Save settings</button></div>
      </form>}
    </>}
  </div>;
}
