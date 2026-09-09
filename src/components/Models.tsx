import { useState } from 'react';
import { Cpu, Play, Square } from 'lucide-react';
import type { Preferences, RuntimeConfig, RuntimeStatus } from '../lib/types';
import { RuntimeForm } from './RuntimeForm';
import { HardwareStatus } from './HardwareStatus';
import { RuntimeDiagnostics } from './RuntimeDiagnostics';
import { ModelDownload } from './ModelDownload';
import { RuntimeDownload } from './RuntimeDownload';
import { open } from '@tauri-apps/plugin-dialog';

interface Props {
  config: RuntimeConfig; preferences: Preferences; runtime: RuntimeStatus; busy: boolean;
  onSaveConfig: (config: RuntimeConfig) => Promise<void>;
  onSavePreferences: (preferences: Preferences) => Promise<void>;
  onLoad: () => void; onUnload: () => void;
}
export function Models({ config, preferences, runtime, busy, onSaveConfig, onSavePreferences, onLoad, onUnload }: Props) {
  const [draft, setDraft] = useState(preferences);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [tab, setTab] = useState<'model' | 'runtime' | 'generation' | 'diagnostics'>('model');
  return <div className="settings-page">
    <div className="page-heading"><p className="eyebrow">ON YOUR MACHINE</p><h1>Models & runtime</h1><p>Make the most of your hardware. Keep control of every response.</p></div>
    <div className="model-status"><div className="model-icon"><Cpu size={24} /></div><div><strong>{runtime.modelPath?.split(/[\\/]/).pop() || 'Your local model'}</strong><p><span className={`status-dot ${runtime.phase === 'ready' ? 'ready' : ''}`} />{runtime.message}</p></div>{runtime.phase === 'ready' ? <button className="secondary" disabled={busy} onClick={onUnload}><Square size={14} />Unload</button> : <button className="primary" disabled={busy || !preferences.modelPath || !preferences.runtimePath} onClick={onLoad}><Play size={14} />{busy ? 'Loading…' : 'Load model'}</button>}</div>
    <HardwareStatus runtime={runtime} />
    <div className="tabs" role="tablist" aria-label="Model settings">{(['model','runtime','generation','diagnostics'] as const).map(value => <button role="tab" aria-selected={tab === value} key={value} onClick={() => { setTab(value); setNotice(''); }}>{value === 'model' ? 'Model files' : value === 'runtime' ? 'Runtime' : value === 'generation' ? 'Generation' : 'Diagnostics'}</button>)}</div>
    {notice && <p className="success" role="status">{notice}</p>}
    {error && <p className="error" role="alert">{error}</p>}
    {tab === 'model' && <ModelDownload busy={busy} onSelect={path => { setDraft(current => ({ ...current, modelPath: path })); setNotice('Verified model selected. Save settings to apply it.'); }} />}
    {tab === 'model' && <RuntimeDownload busy={busy} onSelect={path => { setDraft(current => ({ ...current, runtimePath: path })); setNotice('Installed runtime selected. Save settings to apply it.'); }} />}
    {tab === 'diagnostics' ? <RuntimeDiagnostics /> : tab === 'runtime' ? <RuntimeForm initial={config} busy={busy} onSave={async value => { try { setError(''); await onSaveConfig(value); setNotice('Runtime configuration saved. Reload the model to apply it.'); } catch (e) { setError(String(e)); } }} /> :
      <form className="runtime-form" onSubmit={async event => { event.preventDefault(); try { setError(''); await onSavePreferences(draft); setNotice('Settings saved.'); } catch (e) { setError(String(e)); } }}>
        <fieldset disabled={busy}>{tab === 'model' ? <><legend>Local files</legend><p className="section-description">Use a GGUF model and a compatible llama.cpp runtime. Q6_K is the recommended starting point for MiniCPM5-2B.</p><label>llama-server executable<input aria-label="llama-server executable" placeholder="C:\path\to\llama-server.exe" value={draft.runtimePath} onChange={e => setDraft({ ...draft, runtimePath: e.target.value })} required spellCheck={false} /><small>Choose the CUDA build to use your NVIDIA GPU.</small></label><label>GGUF model file<input aria-label="GGUF model file" placeholder="C:\path\to\MiniCPM5-2B.Q6_K.gguf" value={draft.modelPath} onChange={e => setDraft({ ...draft, modelPath: e.target.value })} required spellCheck={false} /></label></> : <><legend>Response behavior</legend><div className="field-pair"><label>Temperature<input type="number" min="0" max="2" step="0.05" required value={draft.temperature} onChange={e => setDraft({ ...draft, temperature: e.target.valueAsNumber })} /></label><label>Top-p<input type="number" min="0.01" max="1" step="0.01" required value={draft.topP} onChange={e => setDraft({ ...draft, topP: e.target.valueAsNumber })} /></label></div><label>Maximum response tokens<input type="number" min="1" max="32768" required value={draft.maxTokens} onChange={e => setDraft({ ...draft, maxTokens: e.target.valueAsNumber })} /></label><label>System instructions<textarea rows={5} maxLength={32768} value={draft.systemPrompt} onChange={e => setDraft({ ...draft, systemPrompt: e.target.value })} /></label></>}</fieldset>
        {tab === 'model' && <div className="file-actions">{(['runtimePath','modelPath'] as const).map(key => <button type="button" className="secondary" key={key} disabled={busy} onClick={async () => {
          try {
            const path = await open({ multiple: false, directory: false, filters: [{ name: key === 'modelPath' ? 'GGUF models' : 'Executable', extensions: [key === 'modelPath' ? 'gguf' : 'exe'] }] });
            if (path) setDraft(current => ({ ...current, [key]: path }));
          } catch (e) { setError(String(e)); }
        }}>{key === 'modelPath' ? 'Browse model…' : 'Browse runtime…'}</button>)}</div>}
        <div className="form-footer"><small>{tab === 'model' ? 'Model files stay on your computer.' : 'Applies to your next response.'}</small><button className="primary" disabled={busy}>Save settings</button></div>
      </form>}
  </div>;
}
