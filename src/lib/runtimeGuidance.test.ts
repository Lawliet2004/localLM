import { describe, expect, it } from 'vitest';
import { estimateMemory, fittingGpuLayers, modelMaxLayers, recommendationIssue, recommendedConfig } from './runtimeGuidance';
import { defaultRuntimeConfig, type HardwareStatus, type ModelMetadata } from './types';
const model: ModelMetadata = { architecture: 'llama', contextLength: 262144, blockCount: 32, embeddingLength: 4096, headCount: 32, headCountKv: 8, keyLength: null, valueLength: null, fileBytes: 4 * 2 ** 30 };
const gpu = (totalMib: number, usedMib: number): HardwareStatus => ({
  logicalCpus: 12, memoryTotalBytes: 16 * 2 ** 30, memoryAvailableBytes: 8 * 2 ** 30, gpuStatus: 'ok', sampledAt: Date.now(),
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
    expect(estimateMemory({ ...model, architecture: 'hybridnet' }, defaultRuntimeConfig).kvBytes).toBeNull();
    expect(estimateMemory({ ...model, headCountKv: null }, defaultRuntimeConfig).kvBytes).toBeNull();
  });
  it('accounts for projector memory independently of language-model offload', () => {
    const config = { ...defaultRuntimeConfig, contextLength: 4096 };
    const bytes = 2 ** 30;
    const base = estimateMemory(model, config);
    const gpuProjector = estimateMemory(model, config, bytes);
    expect(gpuProjector.gpuBytes).toBe(base.gpuBytes! + bytes);
    expect(gpuProjector.ramBytes).toBe(base.ramBytes);
    const cpuProjector = estimateMemory(model, { ...config, offloadProjector: false }, bytes);
    expect(cpuProjector.gpuBytes).toBe(base.gpuBytes);
    expect(cpuProjector.ramBytes).toBe(base.ramBytes! + bytes);
    expect(estimateMemory(model, { ...config, gpuLayers: 0 }, bytes).gpuBytes).toBe(bytes);
    expect(estimateMemory(model, { ...config, visionEnabled: false }, bytes)).toEqual(base);
    expect(estimateMemory(model, config, null)).toMatchObject({ gpuBytes: null, partial: true });
    expect(estimateMemory(model, { ...config, visionEnabled: false }, null)).toEqual(base);
  });
  it('reserves GPU projector weights when fitting layers and preserves vision choices in recommendations', () => {
    const config = { ...defaultRuntimeConfig, contextLength: 4096 };
    const machine = gpu(6 * 1024, 1024);
    const without = fittingGpuLayers(model, machine, config);
    expect(fittingGpuLayers(model, machine, config, 2 ** 30)).toBeLessThan(without!);
    expect(fittingGpuLayers(model, machine, { ...config, offloadProjector: false }, 2 ** 30)).toBe(without);
    expect(fittingGpuLayers(model, machine, { ...config, visionEnabled: false }, 2 ** 30)).toBe(without);
    expect(recommendedConfig(model, machine, 2 ** 30, { visionEnabled: false, offloadProjector: false })).toMatchObject({ visionEnabled: false, offloadProjector: false });
  });
  it('recommends bounded context and preserves special runtime requirements', () => {
    expect(recommendedConfig({ ...model, contextLength: 2048 }, null).contextLength).toBe(2048);
    expect(recommendedConfig({ ...model, architecture: 'hybridnet' }, null)).toMatchObject({ gpuLayers: -1 });
    expect(recommendedConfig({ ...model, keyLength: 8, valueLength: 8 }, null)).toMatchObject({ cacheTypeK: 'f16', cacheTypeV: 'f16', flashAttention: false });
    expect(recommendedConfig(model, gpu(24 * 1024, 1024)).gpuLayers).toBe(-1);
    expect(recommendedConfig(model, null).gpuLayers).toBe(-1);
  });
  it('separates the model layer count from layers that fit in free VRAM', () => {
    expect(modelMaxLayers(model)).toBe(33);
    const config = { ...defaultRuntimeConfig, contextLength: 8192 };
    expect(fittingGpuLayers(model, gpu(24 * 1024, 1024), config)).toBe(33);
    const tight = fittingGpuLayers(model, gpu(6 * 1024, 1024), config);
    expect(tight).toBeGreaterThan(0);
    expect(tight).toBeLessThan(33);
    expect(fittingGpuLayers({ ...model, architecture: 'hybridnet' }, gpu(24 * 1024, 1024), config)).toBeNull();
    expect(fittingGpuLayers(model, null, config)).toBeNull();
  });
});

it('never offers an oversized configuration as a safe recommendation', () => {
  const machine = { ...gpu(0, 0), gpus: [], memoryTotalBytes: 8 * 2 ** 30, memoryAvailableBytes: 4 * 2 ** 30 };
  const config = recommendedConfig(model, machine);
  expect(config.contextLength).toBe(128);
  expect(recommendationIssue(model, machine, config)).toMatch(/exceeds/);
  const smaller = { ...model, fileBytes: 2 ** 30 };
  const fit = recommendedConfig(smaller, machine);
  expect(recommendationIssue(smaller, machine, fit)).toBeNull();
  expect(recommendationIssue({ ...model, architecture: 'unknown' }, machine, fit)).toMatch(/unknown/);
  expect(recommendationIssue(model, null, fit)).toMatch(/unknown/);
});

it('does not bind automatic placement to GPU zero or stale telemetry', () => {
  const machine = gpu(24 * 1024, 0);
  machine.gpus.push({ ...machine.gpus[0], uuid: 'second' });
  expect(fittingGpuLayers(model, machine, defaultRuntimeConfig)).toBeNull();
  expect(recommendedConfig(model, machine).gpuLayers).toBe(-1);
  machine.sampledAt = Date.now() - 60000;
  expect(recommendationIssue(model, machine, recommendedConfig(model, machine))).toMatch(/unknown/);
});

it('does not disable another backend when the sampled NVIDIA device has no room', () => {
  expect(recommendedConfig(model, gpu(1024, 1024)).gpuLayers).toBe(-1);
});
