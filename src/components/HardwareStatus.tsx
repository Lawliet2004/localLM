import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { HardwareStatus as Hardware, RuntimeStatus } from '../lib/types';

export function HardwareStatus({ runtime }: { runtime: RuntimeStatus }) {
  const [hardware, setHardware] = useState<Hardware | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      try { if (document.visibilityState !== 'hidden') { const value = await api.hardwareStatus(); if (!disposed) { setHardware(value); setError(''); } } }
      catch (e) { if (!disposed) setError(errorMessage(e)); }
      finally { if (!disposed) timer = setTimeout(() => void refresh(), 5000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  const loaded = runtime.loadedConfig;
  return <section className="hardware-status" aria-label="Hardware status">
    <div className="hardware-heading"><strong>Hardware</strong><small>{hardware ? `Updated ${new Date(hardware.sampledAt).toLocaleTimeString()}` : nativeAvailable ? 'Reading device status…' : 'Available in the desktop application'}</small></div>
    {hardware && <>
      <div className="system-memory"><span>{hardware.logicalCpus} logical CPUs</span><span>{hardware.memoryAvailableBytes !== null && hardware.memoryTotalBytes !== null ? `${(hardware.memoryAvailableBytes / 1073741824).toFixed(1)} / ${(hardware.memoryTotalBytes / 1073741824).toFixed(1)} GiB RAM available` : 'RAM readings unavailable'}</span></div>
      {hardware.gpus.map(gpu => <div className="gpu-reading" key={gpu.uuid}><div><strong>{gpu.name}</strong><small>Driver {gpu.driverVersion}</small></div><div><span>{gpu.memoryUsedMib !== null && gpu.memoryTotalMib !== null ? `${gpu.memoryUsedMib.toLocaleString()} / ${gpu.memoryTotalMib.toLocaleString()} MiB VRAM used` : 'VRAM reading unavailable'}</span>{gpu.memoryUsedMib !== null && gpu.memoryTotalMib !== null && gpu.memoryTotalMib > 0 && <progress aria-label={`${gpu.name} VRAM usage`} value={gpu.memoryUsedMib} max={gpu.memoryTotalMib} />}</div><span>{gpu.utilizationPercent === null ? 'Utilization unavailable' : `${gpu.utilizationPercent}% GPU utilization`}</span></div>)}
      {!hardware.gpus.length && <p>{hardware.gpuStatus}</p>}
      <small className="hardware-note">Device-wide readings include other applications. They are not model-specific memory measurements.</small>
    </>}
    {loaded && <p className="loaded-settings">Loaded settings · {loaded.contextLength.toLocaleString()} context · {loaded.gpuLayers === 0 ? 'CPU only requested' : loaded.gpuLayers === -1 ? 'All GPU layers requested' : `${loaded.gpuLayers} GPU layers requested`} · K/V cache {loaded.cacheTypeK} / {loaded.cacheTypeV}</p>}
    {runtime.phase === 'ready' && <p className="loaded-settings">{runtime.gpuOffload ? `Runtime reports ${runtime.gpuOffload.layers} / ${runtime.gpuOffload.totalLayers} model layers offloaded to GPU.` : 'Actual GPU layer count unavailable from this runtime.'} Layer placement does not measure GPU utilization.</p>}
    {error && <p role="status">Telemetry unavailable: {error}</p>}
  </section>;
}
