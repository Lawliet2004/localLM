import { useEffect, useState } from 'react';
import { Cpu, Layers, MemoryStick, Sparkles } from 'lucide-react';
import type { CacheType, HardwareStatus, ModelMetadata, RuntimeConfig, RuntimeStatus } from '../lib/types';
import { api, nativeAvailable } from '../lib/api';
import { availableGpuBytes, contextSafetyLimit, estimateMemory, fittingGpuLayers, gib, gpuLayersLabel, modelMaxLayers, recommendationIssue, recommendedConfig } from '../lib/runtimeGuidance';
import { samePath } from '../lib/pathUtils';

interface Props {
  initial: RuntimeConfig;
  onSave: (config: RuntimeConfig) => void;
  busy: boolean;
  modelPath?: string;
  projectorPath?: string;
  onChooseProjector?: () => void;
  runtime?: RuntimeStatus;
}

function meterMax(used: number | null, available: number | null) {
  if (used == null) return 1;
  if (available == null || available <= 0) return Math.max(used, 1);
  return Math.max(used, available);
}

export function RuntimeForm({ initial, onSave, busy, modelPath, projectorPath, onChooseProjector, runtime }: Props) {
  const [config, setConfig] = useState(initial);
  const [error, setError] = useState('');
  const [model, setModel] = useState<ModelMetadata | null>(null);
  const [hardware, setHardware] = useState<HardwareStatus | null>(null);
  const [metadataError, setMetadataError] = useState('');
  const [projectorBytes, setProjectorBytes] = useState<number | null>(projectorPath ? null : 0);
  const [projectorError, setProjectorError] = useState('');
  useEffect(() => setConfig(initial), [initial]);
  useEffect(() => {
    if (!nativeAvailable || !modelPath) return;
    let disposed = false;
    setModel(null); setMetadataError('');
    void api.readModelMetadata(modelPath).then(value => { if (!disposed) setModel(value); }).catch(e => { if (!disposed) setMetadataError(String(e)); });
    const refresh = () => { void api.hardwareStatus().then(value => { if (!disposed) setHardware(value); }).catch(() => { if (!disposed) setHardware(null); }); };
    refresh(); const timer = setInterval(refresh, 15000);
    return () => { disposed = true; clearInterval(timer); };
  }, [modelPath]);
  useEffect(() => {
    let disposed = false;
    setProjectorError('');
    setProjectorBytes(projectorPath ? null : 0);
    if (projectorPath && nativeAvailable) {
      void api.readModelMetadata(projectorPath).then(value => {
        if (!disposed) setProjectorBytes(value.fileBytes);
      }).catch(error => { if (!disposed) setProjectorError(String(error)); });
    }
    return () => { disposed = true; };
  }, [projectorPath]);
  const maxContext = Math.min(model?.contextLength ?? contextSafetyLimit, contextSafetyLimit);
  const modelLayers = model ? modelMaxLayers(model) : null;
  const maxLayers = modelLayers ?? 999;
  const fittedLayers = model ? fittingGpuLayers(model, hardware, { ...config, gpuLayers: -1 }, projectorBytes) : null;
  const estimateConfig = config.gpuLayers === -1 && fittedLayers != null
    ? { ...config, gpuLayers: modelLayers != null && fittedLayers >= modelLayers ? -1 : fittedLayers }
    : config;
  const estimate = model ? estimateMemory(model, estimateConfig, projectorBytes) : null;
  const recommended = model ? recommendedConfig(model, hardware, projectorBytes, config) : null;
  const gpu = hardware?.gpus.length === 1 ? hardware.gpus[0] : null;
  const freeGpu = gpu?.memoryTotalMib != null && gpu?.memoryUsedMib != null ? (gpu.memoryTotalMib - gpu.memoryUsedMib) * 2 ** 20 : null;
  const planningGpu = availableGpuBytes(hardware);
  function update<K extends keyof RuntimeConfig>(key: K, value: RuntimeConfig[K]) {
    setConfig(current => ({ ...current, [key]: value }));
    setError('');
  }
  const device = config.gpuLayers === 0 ? 'cpu' : config.gpuLayers === -1 ? 'auto' : 'custom';
  const loadedHere = Boolean(runtime?.gpuOffload && runtime.phase === 'ready' && samePath(runtime.modelPath, modelPath));
  const loadedModel = runtime?.phase === 'ready' && samePath(runtime.modelPath, modelPath);
  const vision = config.visionEnabled !== false;
  const projectorOffload = config.offloadProjector !== false;
  return <form className="runtime-form" onSubmit={event => {
    event.preventDefault();
    if (!config.flashAttention && config.cacheTypeV !== 'f16') {
      setError('Quantized value cache requires Flash Attention. Enable it or select F16 for the value cache.');
      return;
    }
    if (config.microBatchSize > config.batchSize) {
      setError('Micro batch size cannot exceed batch size.'); return;
    }
    if (config.contextLength > maxContext) {
      setError('Saved context exceeds this model’s limit. Choose a smaller value before saving.'); return;
    }
    onSave(config);
  }}>
    <section className="runtime-overview" aria-label="Selected model limits">
      <div className="section-heading"><div><h2>Runtime</h2><p className="runtime-model-name">{modelPath?.split(/[\\/]/).pop() || 'Select a model in the Models tab to see its limits'}</p></div><Cpu size={24} /></div>
      {!modelPath && <p className="section-description">Use a downloaded model first. Runtime limits come from that file’s GGUF header and the GPU/RAM available right now.</p>}
      {metadataError && <p className="selection-warning">Model limits unavailable: {metadataError}</p>}
      <div className="runtime-dashboard">
        <article className="runtime-card">
          <div className="runtime-card-head"><Layers size={18} /><div><small>GPU offload</small><strong>{modelLayers ? `${modelLayers} layers in this model` : 'Layer count unknown'}</strong></div></div>
          <dl className="runtime-stats">
            <div><dt>Model maximum</dt><dd>{modelLayers ? `${modelLayers} layers` : 'Unknown'}</dd></div>
            <div><dt>Fits in current VRAM</dt><dd>{fittedLayers == null ? 'Unknown' : fittedLayers === 0 ? 'CPU recommended' : fittedLayers === modelLayers ? 'All layers' : `${fittedLayers} layers`}</dd></div>
            <div><dt>Requested</dt><dd>{gpuLayersLabel(config.gpuLayers)}</dd></div>
            {loadedHere && runtime?.gpuOffload && <div><dt>Currently loaded</dt><dd>{runtime.gpuOffload.layers} / {runtime.gpuOffload.totalLayers} offloaded</dd></div>}
          </dl>
          <p className="runtime-help">Automatic puts as many layers as will fit in free VRAM on the GPU and keeps the rest in system RAM. No GPU, or a model that cannot use this GPU, runs on CPU. A custom count overrides that.</p>
          <label>Compute device<select value={device} disabled={busy} onChange={e => {
            const value = e.target.value;
            if (value === 'cpu') setConfig(current => ({ ...current, gpuLayers: 0, offloadKvCache: false }));
            else if (value === 'auto') setConfig(current => ({ ...current, gpuLayers: -1, offloadKvCache: true }));
            else update('gpuLayers', Math.max(1, Math.min(maxLayers, fittedLayers || 20)));
            setError('');
          }}>
            <option value="auto">Automatic · fill GPU, rest in RAM</option>
            <option value="custom">Custom GPU layer count</option>
            <option value="cpu">CPU only</option>
          </select></label>
          {device === 'custom' && <div className="runtime-slider-row">
            <label>GPU layers<input type="range" min={1} max={maxLayers} disabled={busy} value={Math.min(maxLayers, Math.max(1, config.gpuLayers))} onChange={e => update('gpuLayers', e.target.valueAsNumber)} /></label>
            <label className="runtime-slider-number">Layers<input type="number" min={1} max={maxLayers} required disabled={busy} value={Number.isNaN(config.gpuLayers) ? '' : config.gpuLayers} onChange={e => update('gpuLayers', e.target.valueAsNumber)} /></label>
            <span className="runtime-slider-value">{config.gpuLayers} of {modelLayers ?? maxLayers}</span>
          </div>}
        </article>
        <article className="runtime-card">
          <div className="runtime-card-head"><Sparkles size={18} /><div><small>Context window</small><strong>{model?.contextLength ? `${model.contextLength.toLocaleString()} token model maximum` : 'Context limit unknown'}</strong></div></div>
          <dl className="runtime-stats">
            <div><dt>Full model context</dt><dd>{model?.contextLength ? `${model.contextLength.toLocaleString()} tokens` : 'Unknown'}</dd></div>
            {model?.contextLength && model.contextLength > contextSafetyLimit && <div><dt>Harness safety cap</dt><dd>{contextSafetyLimit.toLocaleString()} tokens</dd></div>}
            <div><dt>Recommended</dt><dd>{recommended && model && !recommendationIssue(model, hardware, recommended, projectorBytes) ? `${recommended.contextLength.toLocaleString()} tokens` : 'Unavailable'}</dd></div>
            <div><dt>Current setting</dt><dd>{config.contextLength.toLocaleString()} tokens</dd></div>
          </dl>
          <p className="runtime-help">Training context from the GGUF header. Larger windows need more attention-cache memory. Loading refuses a context beyond the known model or harness limit.</p>
          <label>Context window<input type="number" min={128} max={maxContext} required disabled={busy} value={Number.isNaN(config.contextLength) ? '' : config.contextLength} onChange={e => update('contextLength', e.target.valueAsNumber)} />
            {model?.contextLength && <input aria-label="Context window slider" type="range" min={128} max={maxContext} disabled={busy} value={Math.min(maxContext, Math.max(128, config.contextLength))} onChange={e => update('contextLength', e.target.valueAsNumber)} />}
          </label>
          <div className="context-presets">{[2048, 8192, 32768].filter(n => n <= maxContext).map(n => <button type="button" className="secondary" key={n} onClick={() => update('contextLength', n)}>{n.toLocaleString()}</button>)}{model?.contextLength && <button type="button" className="secondary" onClick={() => update('contextLength', maxContext)}>Use full context{maxContext < model.contextLength ? ' (capped)' : ''}</button>}</div>
          {config.contextLength > maxContext && <p className="error" role="alert">Saved context exceeds this model’s limit. Choose a smaller value before saving.</p>}
        </article>
        <article className="runtime-card runtime-card-wide">
          <div className="runtime-card-head"><Sparkles size={18} /><div><small>Optional model component</small><strong>Vision projector</strong></div></div>
          <p className="runtime-help">Keep the matching projector downloaded and choose whether to load it. Its placement is independent of the language model's GPU layers.</p>
          <label className="check-row"><input type="checkbox" checked={vision} disabled={busy} onChange={e => update('visionEnabled', e.target.checked)} />Load vision projector</label>
          <label>Projector placement<select aria-label="Projector placement" value={projectorOffload ? 'gpu' : 'cpu'} disabled={busy || !vision} onChange={e => update('offloadProjector', e.target.value === 'gpu')}>
            <option value="gpu">GPU preferred · use VRAM</option>
            <option value="cpu">CPU only · use system RAM</option>
          </select></label>
          <dl className="runtime-stats">
            <div><dt>Configured file</dt><dd className="runtime-projector-file" title={projectorPath}>{projectorPath?.split(/[\\/]/).pop() || 'No projector configured'}</dd></div>
            <div><dt>Projector weights</dt><dd>{!projectorPath ? 'None' : projectorBytes === null ? 'Size unknown' : gib(projectorBytes)}</dd></div>
            <div><dt>After saving</dt><dd>{!vision || !projectorPath ? 'Off · text only' : projectorOffload ? 'On · GPU preferred' : 'On · CPU only'}</dd></div>
            {loadedModel && <div><dt>Currently loaded</dt><dd>{runtime.projector ? `On · ${runtime.projector.offload ? 'GPU preferred' : 'CPU only'}` : runtime.projector === null ? 'Off · text only' : 'Status unavailable; reload to check'}</dd></div>}
            {loadedModel && runtime.projector && <div><dt>Loaded file</dt><dd className="runtime-projector-file" title={runtime.projector.path}>{runtime.projector.path.split(/[\\/]/).pop()}</dd></div>}
          </dl>
          {onChooseProjector && <button type="button" className="secondary" disabled={busy} onClick={onChooseProjector}>Choose projector file…</button>}
          {vision && !projectorPath && <p className="runtime-help">Choose a matching mmproj file to enable images. Until then the model loads with text input only.</p>}
          {vision && projectorError && <p className="selection-warning">Projector unavailable: {projectorError}. Choose an existing matching file, or turn off Load vision projector.</p>}
          <p className="runtime-help">Turning this off keeps the file and frees its runtime memory after reload. CPU placement keeps image input while saving projector VRAM. GPU preferred allows acceleration when supported. These are startup settings: saving reloads the model and clears its in-memory prompt cache; conversation history stays saved. Projector layers cannot be split individually with these runtime controls.</p>
        </article>
        <article className="runtime-card runtime-card-wide">
          <div className="runtime-card-head"><MemoryStick size={18} /><div><small>Memory this configuration will use</small><strong>{model ? gib(model.fileBytes) : 'Unknown'} weights on disk</strong></div></div>
          {estimate ? <div className="memory-meters" aria-label="Estimated model memory">
            <div className={estimate.gpuBytes != null && freeGpu != null && estimate.gpuBytes + 2 ** 30 > freeGpu ? 'tight' : undefined}>
              <div className="memory-meter-head"><small>GPU · weights + cache + projector</small><strong>{estimate.gpuBytes == null ? 'Unknown' : gib(estimate.gpuBytes)}</strong></div>
              <progress aria-label="Estimated GPU memory" value={estimate.gpuBytes ?? 0} max={meterMax(estimate.gpuBytes, freeGpu)} />
              <span>{freeGpu == null ? 'Available VRAM unknown' : `${gib(freeGpu)} VRAM free now`}</span>
            </div>
            <div className={estimate.ramBytes != null && hardware?.memoryAvailableBytes != null && estimate.ramBytes + 2 ** 30 > hardware.memoryAvailableBytes ? 'tight' : undefined}>
              <div className="memory-meter-head"><small>System RAM · weights + cache + projector</small><strong>{estimate.ramBytes == null ? 'Unknown' : gib(estimate.ramBytes)}</strong></div>
              <progress aria-label="Estimated system memory" value={estimate.ramBytes ?? 0} max={meterMax(estimate.ramBytes, hardware?.memoryAvailableBytes ?? null)} />
              <span>{hardware?.memoryAvailableBytes == null ? 'Available RAM unknown' : `${gib(hardware.memoryAvailableBytes)} RAM free now`}</span>
            </div>
            <div>
              <div className="memory-meter-head"><small>Attention cache</small><strong>{estimate.kvBytes == null ? 'Unknown' : gib(estimate.kvBytes)}</strong></div>
              <span>At {config.contextLength.toLocaleString()} context tokens{estimate.partial ? ' · cache layout incomplete' : ''}</span>
            </div>
          </div> : <p className="runtime-help">Memory estimates appear after the selected GGUF header can be read.</p>}
          <p className="runtime-help">Planning estimates, not measured allocation. {estimate?.partial ? 'Cache layout, layer count, or projector size is unavailable, so GPU/RAM totals are incomplete. ' : ''}Projector weight size is charged to its selected device. Layer sizes vary. Image/compute buffers, runtime overhead and memory-mapped pages are excluded. About 1 GiB of GPU headroom is reserved in the “fits in VRAM” figure{planningGpu != null ? ` (${gib(planningGpu)} usable after that reserve)` : ''}.</p>
          {estimate?.gpuBytes != null && freeGpu !== null && estimate.gpuBytes + 2 ** 30 > freeGpu && <p className="selection-warning">GPU memory is tight. Reduce GPU layers or context, or move the cache to RAM. Other running models are included in the available-memory reading.</p>}
          {estimate?.ramBytes != null && hardware?.memoryAvailableBytes != null && estimate.ramBytes + 2 ** 30 > hardware.memoryAvailableBytes && <p className="selection-warning">System RAM is tight. Reduce context or choose a smaller quantization.</p>}
        </article>
      </div>
      {recommended && <div className="runtime-recommendation"><div>
        <strong>Suggested starting settings</strong>{model && recommendationIssue(model, hardware, recommended, projectorBytes) && <p role="status">{recommendationIssue(model, hardware, recommended, projectorBytes)}</p>}
        <p>Uses a conservative RAM budget with room for cache, compute buffers and the operating system. GPU selection remains automatic when device capability is unknown.</p>
        <dl className="runtime-compare">
          <div><dt /><dd>Recommended</dd><dd>Current</dd></div>
          <div><dt>Context</dt><dd>{recommended.contextLength.toLocaleString()}</dd><dd>{config.contextLength.toLocaleString()}</dd></div>
          <div><dt>GPU offload</dt><dd>{gpuLayersLabel(recommended.gpuLayers)}</dd><dd>{gpuLayersLabel(config.gpuLayers)}</dd></div>
          <div><dt>CPU threads</dt><dd>{recommended.cpuThreads}</dd><dd>{config.cpuThreads}</dd></div>
          <div><dt>K/V cache</dt><dd>{recommended.cacheTypeK.toUpperCase()} / {recommended.cacheTypeV.toUpperCase()}</dd><dd>{config.cacheTypeK.toUpperCase()} / {config.cacheTypeV.toUpperCase()}</dd></div>
          <div><dt>Flash Attention</dt><dd>{recommended.flashAttention ? 'On' : 'Off'}</dd><dd>{config.flashAttention ? 'On' : 'Off'}</dd></div>
        </dl>
      </div><button type="button" className="secondary" disabled={busy || Boolean(model && recommendationIssue(model, hardware, recommended, projectorBytes))} onClick={() => { setConfig(recommended); setError(''); }}>Use recommended settings</button></div>}
    </section>
    <details className="runtime-advanced">
      <summary>Advanced memory and batch settings</summary>
      <fieldset disabled={busy}>
        <legend>Memory & attention</legend>
        <label className="check-row"><input type="checkbox" checked={config.flashAttention} onChange={e => update('flashAttention', e.target.checked)} />Flash Attention</label>
        <p className="section-description">Flash Attention can reduce memory use and speed up long prompts. Support depends on the architecture and runtime. Quantized value caches require it.</p>
        <div className="field-pair">{(['cacheTypeK', 'cacheTypeV'] as const).map(key => <label key={key}>{key === 'cacheTypeK' ? 'Key cache precision' : 'Value cache precision'}<select value={config[key]} onChange={e => update(key, e.target.value as CacheType)}><option value="f16">F16 · full cache precision</option><option value="q8_0">Q8 · balanced</option><option value="q4_0">Q4 · less memory</option></select></label>)}</div>
        <label className="check-row"><input type="checkbox" checked={config.offloadKvCache} onChange={e => update('offloadKvCache', e.target.checked)} />Keep KV cache on GPU</label>
        <label className="check-row"><input type="checkbox" checked={config.mmap} onChange={e => update('mmap', e.target.checked)} />Memory-map model file</label>
        <p className="section-description">The KV cache stores attention history. Q8 uses roughly half the F16 cache memory; Q4 uses roughly a quarter, with a greater quality tradeoff. Memory mapping lets the OS page weights from disk; it does not eliminate memory needs.</p>
        <label>CPU threads<input type="number" min={1} max={256} required value={Number.isNaN(config.cpuThreads) ? '' : config.cpuThreads} onChange={e => update('cpuThreads', e.target.valueAsNumber)} /></label>
        <label>Inference slots<select aria-label="Inference slots" value={config.inferenceSlots ?? 1} onChange={e => update('inferenceSlots', Number(e.target.value))}>
          <option value={1}>1 · default (4 GB)</option>
          <option value={2}>2 · keep parent cache during subagents</option>
        </select></label>
        <p className="section-description">Two slots keep the parent conversation&apos;s prompt cache when a subagent runs. They also roughly double KV-cache memory. Keep 1 on 4 GB GPUs.</p>
      </fieldset>
      <fieldset disabled={busy}>
        <legend>Prompt processing</legend>
        <p className="section-description">Batch size limits tokens processed together. Micro batch size limits each compute chunk. Smaller chunks need less working memory; larger ones may process prompts faster.</p>
        <div className="field-pair"><label>Batch size<input type="number" min={1} max={8192} required value={Number.isNaN(config.batchSize) ? '' : config.batchSize} onChange={e => update('batchSize', e.target.valueAsNumber)} /></label><label>Micro batch size<input type="number" min={1} max={8192} required value={Number.isNaN(config.microBatchSize) ? '' : config.microBatchSize} onChange={e => update('microBatchSize', e.target.valueAsNumber)} /></label></div>
      </fieldset>
    </details>
    {error && <p className="error" role="alert">{error}</p>}
    <div className="form-footer"><small>{runtime?.phase === 'ready' ? 'Saving reloads the model to apply context, memory, and projector settings.' : 'Load the model after saving to apply context, memory, and projector settings.'}</small><button className="primary" type="submit" disabled={busy}>Save configuration</button></div>
  </form>;
}
