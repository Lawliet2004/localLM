import { describe, expect, it } from 'vitest';
import { downloadSize, fitKind, hubSize, isWeightFile, parseHubQuery, preferredFile, preferredProjector, projectorFiles, quantLabel, repoParts, weightFiles } from './modelStudio';
import type { HardwareStatus } from './types';

const hardware = (vramMib: number, ramBytes: number): HardwareStatus => ({
  logicalCpus: 8, memoryTotalBytes: ramBytes, memoryAvailableBytes: ramBytes / 2,
  gpuStatus: 'ok', sampledAt: 0,
  gpus: [{ name: 'GPU', uuid: 'g', memoryUsedMib: 0, memoryTotalMib: vramMib, utilizationPercent: 0, driverVersion: '1' }],
});

describe('model studio helpers', () => {
  it('accepts Hugging Face URLs and owner/repo strings', () => {
    expect(parseHubQuery('https://huggingface.co/bartowski/Qwen2.5-7B-Instruct-GGUF')).toBe('bartowski/Qwen2.5-7B-Instruct-GGUF');
    expect(parseHubQuery('owner/model-GGUF')).toBe('owner/model-GGUF');
    expect(parseHubQuery('qwen 7b')).toBeNull();
  });

  it('reads quantization tags and prefers Q4_K_M', () => {
    expect(quantLabel('folder/Model-Q5_K_M.gguf')).toBe('Q5_K_M');
    expect(quantLabel('Model-Q2_0.gguf')).toBe('Q2_0');
    expect(quantLabel('model-bf16.gguf')).toBe('BF16');
    expect(preferredFile([
      { filename: 'model-Q8_0.gguf', bytes: 8, sha256: 'a' },
      { filename: 'model-Q4_K_M.gguf', bytes: 4, sha256: 'b' },
      { filename: 'mmproj-f16.gguf', bytes: 1, sha256: 'c' },
    ])).toBe('model-Q4_K_M.gguf');
  });

  it('uses available memory and runtime headroom for memory candidates', () => {
    const machine = hardware(4096, 16 * 2 ** 30);
    expect(fitKind(2 * 2 ** 30, machine)).toBe('gpu');
    expect(fitKind(6 * 2 ** 30, machine)).toBe('ram');
    expect(fitKind(8 * 2 ** 30, machine)).toBe('large');
    expect(fitKind(14 * 2 ** 30, machine)).toBe('large');
    expect(repoParts('bartowski/Qwen2.5-7B-Instruct-GGUF')).toEqual({
      publisher: 'bartowski', name: 'Qwen2.5 7B Instruct', id: 'bartowski/Qwen2.5-7B-Instruct-GGUF',
    });
  });

  it('includes the projector and treats missing memory telemetry as unknown', () => {
    const machine = hardware(4096, 16 * 2 ** 30);
    expect(fitKind(2 * 2 ** 30, machine, 1.5 * 2 ** 30)).toBe('ram');
    expect(fitKind(2 * 2 ** 30, { ...machine, gpus: [{ ...machine.gpus[0], memoryUsedMib: 2048 }] })).toBe('ram');
    expect(fitKind(2 * 2 ** 30, { ...machine, memoryAvailableBytes: null, gpus: [{ ...machine.gpus[0], memoryUsedMib: null }] })).toBe('unknown');
    expect(fitKind(2 * 2 ** 30, null)).toBe('unknown');
    expect(fitKind(0, machine)).toBe('unknown');
  });

  it('excludes companion files without hiding full models that embed MTP', () => {
    const files = [
      'gemma-4-E4B-it-Q4_K_M.gguf',
      'gemma-4-E4B-it-Q4_K_M-00001-of-00002.gguf',
      'gemma-4-E4B-it-Q4_K_M-00002-of-00002.gguf',
      'mtp-gemma-4-E4B-it.gguf',
      'MTP/mtp-gemma-4-E4B-it-Q8_0.gguf',
      'MTP/gemma-4-E4B-it-Q4_K_M.gguf',
      'mmproj-BF16.gguf',
      'imatrix_unsloth.gguf',
      'adapters/custom.gguf',
      'lora-model.gguf',
      'README.md',
      'Qwen3.5-27B-MTP-Q4_K_M.gguf',
    ].map(filename => ({ filename, bytes: 100, sha256: 'a' }));
    expect(weightFiles(files).map(file => file.filename)).toEqual([
      'gemma-4-E4B-it-Q4_K_M.gguf',
      'gemma-4-E4B-it-Q4_K_M-00001-of-00002.gguf',
      'Qwen3.5-27B-MTP-Q4_K_M.gguf',
    ]);
    expect(preferredFile(files)).toBe('gemma-4-E4B-it-Q4_K_M.gguf');
    expect(downloadSize(files, files[1].filename)).toBe(200);
    expect(isWeightFile('folder/MTP-MODEL.GGUF')).toBe(false);
  });

  it('converts the actual Gemma file byte counts into Hugging Face decimal units', () => {
    expect(hubSize(4_977_171_584)).toBe('4.98 GB');
    expect(hubSize(991_552_320)).toBe('991.55 MB');
    expect(hubSize(1_912_464_192)).toBe('1.91 GB');
  });

  it('finds vision projectors and prefers the matching quant stem', () => {
    const files = [
      { filename: 'BAAI_AREX-Turbo-Q4_K_M.gguf', bytes: 4, sha256: 'b' },
      { filename: 'mmproj-BAAI_AREX-Turbo-Q8_0.gguf', bytes: 2, sha256: 'c' },
      { filename: 'mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf', bytes: 1, sha256: 'd' },
      { filename: 'README.md', bytes: 1, sha256: 'e' },
    ];
    expect(projectorFiles(files).map(file => file.filename)).toEqual([
      'mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf',
      'mmproj-BAAI_AREX-Turbo-Q8_0.gguf',
    ]);
    expect(preferredProjector(files, 'BAAI_AREX-Turbo-Q4_K_M.gguf')).toBe('mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf');
    expect(preferredProjector(files, 'other.gguf')).toBe('');
    expect(preferredProjector([{ filename: 'model.gguf', bytes: 1, sha256: 'a' }], 'model.gguf')).toBe('');
  });
});
