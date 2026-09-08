import { useState } from 'react';
import type { CacheType, RuntimeConfig } from '../lib/types';

interface Props {
  initial: RuntimeConfig;
  onSave: (config: RuntimeConfig) => void;
  busy: boolean;
}

export function RuntimeForm({ initial, onSave, busy }: Props) {
  const [config, setConfig] = useState(initial);
  const [error, setError] = useState('');
  function update<K extends keyof RuntimeConfig>(key: K, value: RuntimeConfig[K]) {
    setConfig(current => ({ ...current, [key]: value }));
    setError('');
  }
  const device = config.gpuLayers === 0 ? 'cpu' : config.gpuLayers === -1 ? 'gpu' : 'custom';
  return <form className="runtime-form" onSubmit={event => {
    event.preventDefault();
    if (!config.flashAttention && config.cacheTypeV !== 'f16') {
      setError('Quantized value cache requires Flash Attention. Enable it or select F16 for the value cache.');
      return;
    }
    if (config.microBatchSize > config.batchSize) {
      setError('Micro batch size cannot exceed batch size.'); return;
    }
    onSave(config);
  }}>
    <fieldset disabled={busy}>
      <legend>Compute</legend>
      <label>Compute device<select value={device} onChange={e => update('gpuLayers', e.target.value === 'cpu' ? 0 : e.target.value === 'gpu' ? -1 : 20)}>
        <option value="gpu">GPU · all layers</option><option value="custom">GPU + CPU · custom</option><option value="cpu">CPU only</option>
      </select></label>
      {device === 'custom' && <label>GPU layers<input type="number" min="1" max="999" required value={config.gpuLayers} onChange={e => update('gpuLayers', e.target.valueAsNumber)} /></label>}
      <label>Context window<input type="number" min="128" max="131072" required value={config.contextLength} onChange={e => update('contextLength', e.target.valueAsNumber)} /><small>Shared by your messages, tool results and model response.</small></label>
      <label>CPU threads<input type="number" min="1" max="256" required value={config.cpuThreads} onChange={e => update('cpuThreads', e.target.valueAsNumber)} /></label>
    </fieldset>
    <fieldset disabled={busy}>
      <legend>Memory & attention</legend>
      <label className="check-row"><input type="checkbox" checked={config.flashAttention} onChange={e => update('flashAttention', e.target.checked)} />Flash Attention</label>
      <div className="field-pair">{(['cacheTypeK', 'cacheTypeV'] as const).map(key => <label key={key}>{key === 'cacheTypeK' ? 'Key cache precision' : 'Value cache precision'}<select value={config[key]} onChange={e => update(key, e.target.value as CacheType)}><option value="f16">F16 · full cache precision</option><option value="q8_0">Q8 · balanced</option><option value="q4_0">Q4 · less memory</option></select></label>)}</div>
      <label className="check-row"><input type="checkbox" checked={config.offloadKvCache} onChange={e => update('offloadKvCache', e.target.checked)} />Keep KV cache on GPU</label>
      <label className="check-row"><input type="checkbox" checked={config.mmap} onChange={e => update('mmap', e.target.checked)} />Memory-map model file</label>
    </fieldset>
    <fieldset disabled={busy}>
      <legend>Prompt processing</legend>
      <div className="field-pair"><label>Batch size<input type="number" min="1" max="8192" required value={config.batchSize} onChange={e => update('batchSize', e.target.valueAsNumber)} /></label><label>Micro batch size<input type="number" min="1" max="8192" required value={config.microBatchSize} onChange={e => update('microBatchSize', e.target.valueAsNumber)} /></label></div>
    </fieldset>
    {error && <p className="error" role="alert">{error}</p>}
    <div className="form-footer"><small>Changes apply the next time you load the model.</small><button className="primary" type="submit" disabled={busy}>Save configuration</button></div>
  </form>;
}
