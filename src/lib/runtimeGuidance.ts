import { defaultRuntimeConfig, type HardwareStatus, type ModelMetadata, type RuntimeConfig } from './types';
export const gib = (bytes: number) => `${(bytes / 2 ** 30).toFixed(2)} GiB`;
export const contextSafetyLimit = 2_097_152;

export function modelMaxLayers(model: ModelMetadata) {
  return model.blockCount ? model.blockCount + 1 : null;
}

export function gpuLayersLabel(layers: number) {
  if (layers === -1) return 'Automatic · fill GPU, rest in RAM';
  if (layers === 0) return 'CPU only';
  return `${layers} GPU layers`;
}

export function estimateMemory(model: ModelMetadata, config: RuntimeConfig, projectorBytes: number | null = 0) {
  const blocks = model.blockCount;
  const headSize = model.embeddingLength && model.headCount ? model.embeddingLength / model.headCount : null;
  // Standard attention only. Hybrid/recurrent/MLA caches require architecture-specific formulas.
  const standard = ['llama', 'qwen2', 'qwen3', 'gemma', 'gemma2', 'mistral', 'phi3'].includes(model.architecture ?? '');
  const key = model.keyLength ?? headSize;
  const value = model.valueLength ?? headSize;
  const bytes = { f16: 2, q8_0: 34 / 32, q4_0: 18 / 32 };
  const slots = Math.max(1, config.inferenceSlots || 1);
  const kvBytes = standard && blocks && model.headCountKv && key && value
    ? config.contextLength * blocks * model.headCountKv * (key * bytes[config.cacheTypeK] + value * bytes[config.cacheTypeV]) * slots : null;
  const fraction = config.gpuLayers === 0 ? 0 : config.gpuLayers === -1 ? 1 : blocks ? Math.min(1, config.gpuLayers / (blocks + 1)) : null;
  const gpuWeights = fraction === null ? null : model.fileBytes * fraction;
  const gpuCache = fraction === null || kvBytes === null ? null : config.offloadKvCache ? kvBytes * fraction : 0;
  const projector = config.visionEnabled === false ? 0 : projectorBytes;
  const gpuProjector = config.offloadProjector === false ? 0 : projector;
  const ramProjector = config.offloadProjector === false ? projector : 0;
  const gpuBase = fraction === 0 ? 0 : gpuWeights !== null ? gpuWeights + (gpuCache ?? 0) : null;
  const ramBase = fraction === null ? null : model.fileBytes * (1 - fraction) + (kvBytes === null ? 0 : kvBytes - (gpuCache ?? 0));
  return {
    kvBytes, gpuBytes: gpuBase === null || gpuProjector === null ? null : gpuBase + gpuProjector,
    ramBytes: ramBase === null || ramProjector === null ? null : ramBase + ramProjector,
    partial: kvBytes === null || fraction === null || projector === null,
  };
}

export function freshHardware(hardware: HardwareStatus | null) {
  return hardware && Number.isFinite(hardware.sampledAt) && Date.now() - hardware.sampledAt <= 30000 && Date.now() >= hardware.sampledAt ? hardware : null;
}

export function availableGpuBytes(hardware: HardwareStatus | null) {
  hardware = freshHardware(hardware);
  // Telemetry cannot identify the runtime's selected device on multi-GPU hosts.
  const gpu = hardware?.gpus.length === 1 ? hardware.gpus[0] : null;
  if (!gpu || gpu.memoryTotalMib === null || gpu.memoryUsedMib === null) return null;
  return Math.max(0, (gpu.memoryTotalMib - gpu.memoryUsedMib) * 2 ** 20 - 1024 ** 3);
}

export function fittingGpuLayers(model: ModelMetadata, hardware: HardwareStatus | null, config: RuntimeConfig, projectorBytes: number | null = 0) {
  const max = modelMaxLayers(model);
  if (max == null) return null;
  const free = availableGpuBytes(hardware);
  const projector = config.visionEnabled !== false && config.offloadProjector !== false ? projectorBytes : 0;
  if (free === null || projector === null) return null;
  const available = Math.max(0, free - projector);
  const full = estimateMemory(model, { ...config, gpuLayers: -1 });
  if (full.partial || full.gpuBytes == null || full.gpuBytes <= 0) return null;
  if (available >= full.gpuBytes) return max;
  return Math.max(0, Math.floor((available / full.gpuBytes) * max));
}

export function recommendedConfig(model: ModelMetadata, hardware: HardwareStatus | null, projectorBytes: number | null = 0, vision?: Pick<RuntimeConfig, 'visionEnabled' | 'offloadProjector'>): RuntimeConfig {
  const config = { ...defaultRuntimeConfig, visionEnabled: vision?.visionEnabled ?? true, offloadProjector: vision?.offloadProjector ?? true, contextLength: Math.max(128, Math.min(model.contextLength ?? 4096, 8192, contextSafetyLimit)),
    cpuThreads: Math.max(1, Math.min(256, Math.floor((hardware?.logicalCpus ?? 12) / 2))), batchSize: 512, microBatchSize: 128 };
  const headSize = model.embeddingLength && model.headCount ? model.embeddingLength / model.headCount : null;
  const key = model.keyLength ?? headSize, value = model.valueLength ?? headSize;
  if (!key || !value || key % 32 !== 0 || value % 32 !== 0) {
    config.cacheTypeK = 'f16'; config.cacheTypeV = 'f16'; config.flashAttention = false;
  }
  // Bound against host RAM too: mmap, staging and backend buffers can retain
  // weights even with GPU offload. Unknown cache/projector estimates stay unknown.
  const available = freshHardware(hardware)?.memoryAvailableBytes;
  const reserve = Math.max(2 ** 30, model.fileBytes * 0.15);
  while (available != null && config.contextLength > 128) {
    const estimate = estimateMemory(model, { ...config, gpuLayers: 0, offloadKvCache: false, offloadProjector: false }, projectorBytes);
    if (!estimate.partial && estimate.ramBytes != null && estimate.ramBytes + reserve <= available * 0.8) break;
    config.contextLength = Math.max(128, Math.floor(config.contextLength / 2 / 128) * 128);
  }
  // Driver telemetry cannot prove which backend/device the selected runtime uses.
  // Leave placement automatic even when a separately sampled NVIDIA GPU is full.
  config.gpuLayers = -1;
  config.offloadKvCache = true;
  return config;
}

export function recommendationIssue(model: ModelMetadata, hardware: HardwareStatus | null, config: RuntimeConfig, projectorBytes: number | null = 0): string | null {
  const estimate = estimateMemory(model, { ...config, gpuLayers: 0, offloadKvCache: false, offloadProjector: false }, projectorBytes);
  hardware = freshHardware(hardware);
  if (hardware?.memoryAvailableBytes == null || estimate.partial || estimate.ramBytes == null)
    return 'Memory fit is unknown. Review the runtime and memory requirements before loading.';
  if (estimate.ramBytes + Math.max(2 ** 30, model.fileBytes * 0.15) > hardware.memoryAvailableBytes * 0.8)
    return 'This model exceeds the conservative memory budget even at minimum context. Choose a smaller model or free memory.';
  return null;
}
