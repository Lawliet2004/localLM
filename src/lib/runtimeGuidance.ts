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

export function estimateMemory(model: ModelMetadata, config: RuntimeConfig) {
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
  return {
    kvBytes, gpuBytes: fraction === 0 ? 0 : gpuWeights !== null ? gpuWeights + (gpuCache ?? 0) : null,
    ramBytes: fraction === null ? null : model.fileBytes * (1 - fraction) + (kvBytes === null ? 0 : kvBytes - (gpuCache ?? 0)),
    partial: kvBytes === null || fraction === null,
  };
}

export function availableGpuBytes(hardware: HardwareStatus | null) {
  const gpu = hardware?.gpus[0];
  if (!gpu || gpu.memoryTotalMib === null || gpu.memoryUsedMib === null) return null;
  return Math.max(0, (gpu.memoryTotalMib - gpu.memoryUsedMib) * 2 ** 20 - 1024 ** 3);
}

export function fittingGpuLayers(model: ModelMetadata, hardware: HardwareStatus | null, config: RuntimeConfig) {
  const max = modelMaxLayers(model);
  if (max == null) return null;
  const available = availableGpuBytes(hardware);
  if (available === null) return 0;
  const full = estimateMemory(model, { ...config, gpuLayers: -1 });
  if (full.partial || full.gpuBytes == null || full.gpuBytes <= 0) return 0;
  if (available >= full.gpuBytes) return max;
  return Math.max(0, Math.floor((available / full.gpuBytes) * max));
}

export function recommendedConfig(model: ModelMetadata, hardware: HardwareStatus | null): RuntimeConfig {
  const config = { ...defaultRuntimeConfig, contextLength: Math.max(128, Math.min(model.contextLength ?? 131072, contextSafetyLimit)),
    cpuThreads: Math.max(1, Math.min(256, Math.floor((hardware?.logicalCpus ?? 12) / 2))), batchSize: 512, microBatchSize: 128 };
  const headSize = model.embeddingLength && model.headCount ? model.embeddingLength / model.headCount : null;
  const key = model.keyLength ?? headSize, value = model.valueLength ?? headSize;
  if (!key || !value || key % 32 !== 0 || value % 32 !== 0) {
    config.cacheTypeK = 'f16'; config.cacheTypeV = 'f16'; config.flashAttention = false;
  }
  // Use currently available VRAM conservatively; leave room for compute buffers and the desktop.
  const fitted = fittingGpuLayers(model, hardware, config);
  if (fitted === null || fitted === 0) {
    return { ...config, gpuLayers: 0, offloadKvCache: false };
  }
  config.gpuLayers = -1;
  config.offloadKvCache = true;
  return config;
}
