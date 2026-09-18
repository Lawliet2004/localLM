import { describe, expect, it } from 'vitest';
import { estimateMemory, fittingGpuLayers, modelMaxLayers, recommendedConfig } from './runtimeGuidance';
import { defaultRuntimeConfig, type HardwareStatus, type ModelMetadata } from './types';
const model: ModelMetadata = { architecture: 'llama', contextLength: 262144, blockCount: 32, embeddingLength: 4096, headCount: 32, headCountKv: 8, keyLength: null, valueLength: null, fileBytes: 4 * 2 ** 30 };
const gpu = (totalMib: number, usedMib: number): HardwareStatus => ({
  logicalCpus: 12, memoryTotalBytes: 16 * 2 ** 30, memoryAvailableBytes: 8 * 2 ** 30, gpuStatus: 'ok', sampledAt: 0,
  gpus: [{ name: 'GPU', uuid: 'gpu', memoryUsedMib: usedMib, memoryTotalMib: totalMib, utilizationPercent: 0, driverVersion: '1' }],
});
describe('runtime memory guidance', () => {
  it('scales the KV cache with context and precision and places CPU cache in RAM', () => {
    const config = { ...defaultRuntimeConfig, contextLength: 4096, cacheTypeK: 'f16' as const, cacheTypeV: 'f16' as const };
    const short = estimateMemory(model, config);
    const long = estimateMemory(model, { ...config, contextLength: 8192 });
    expect(long.kvBytes).toBe(short.kvBytes! * 2);
    expect(estimateMemory(model, { ...config, gpuLayers: 0 }).gpuBytes).toBe(0);
    expect(estimateMemory(model, { ...config, cacheTypeK: 'q8_0', cacheTypeV: 'q8_0' }).kvBytes).toBeLessThan(short.kvBytes!);
    expect(estimateMemory(model, { ...config, inferenceSlots: 2 }).kvBytes).toBe(short.kvBytes! * 2);
  });
  it('does not invent cache estimates for unknown or hybrid architectures', () => {
    expect(estimateMemory({ ...model, architecture: 'zaya' }, defaultRuntimeConfig).kvBytes).toBeNull();
    expect(estimateMemory({ ...model, headCountKv: null }, defaultRuntimeConfig).kvBytes).toBeNull();
  });
  it('recommends bounded context and preserves special runtime requirements', () => {
    expect(recommendedConfig({ ...model, contextLength: 2048 }, null).contextLength).toBe(2048);
    expect(recommendedConfig({ ...model, architecture: 'zaya' }, null)).toMatchObject({ gpuLayers: 0, flashAttention: false, cacheTypeV: 'f16' });
    expect(recommendedConfig({ ...model, keyLength: 8, valueLength: 8 }, null)).toMatchObject({ cacheTypeK: 'f16', cacheTypeV: 'f16', flashAttention: false });
    expect(recommendedConfig(model, gpu(24 * 1024, 1024)).gpuLayers).toBe(-1);
    expect(recommendedConfig(model, null).gpuLayers).toBe(0);
  });
  it('separates the model layer count from layers that fit in free VRAM', () => {
    expect(modelMaxLayers(model)).toBe(33);
    const config = { ...defaultRuntimeConfig, contextLength: 8192 };
    expect(fittingGpuLayers(model, gpu(24 * 1024, 1024), config)).toBe(33);
    const tight = fittingGpuLayers(model, gpu(6 * 1024, 1024), config);
    expect(tight).toBeGreaterThan(0);
    expect(tight).toBeLessThan(33);
    expect(fittingGpuLayers({ ...model, architecture: 'zaya' }, gpu(24 * 1024, 1024), config)).toBe(0);
    expect(fittingGpuLayers(model, null, config)).toBe(0);
  });
});
