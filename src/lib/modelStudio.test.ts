import { describe, expect, it } from 'vitest';
import { fitKind, isBonsai2Filename, parseHubQuery, preferredFile, preferredProjector, projectorFiles, quantLabel, repoParts } from './modelStudio';
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
    expect(quantLabel('Ternary-Bonsai-2-27B-PTQ1_0.gguf')).toBe('PTQ1_0');
    expect(quantLabel('Ternary-Bonsai-2-27B-PQ2_0.gguf')).toBe('PQ2_0');
    expect(isBonsai2Filename('Ternary-Bonsai-2-27B-PTQ1_0.gguf')).toBe(true);
    expect(isBonsai2Filename('Ternary-Bonsai-8B-Q2_0.gguf')).toBe(false);
    expect(preferredFile([
      { filename: 'model-Q8_0.gguf', bytes: 8, sha256: 'a' },
      { filename: 'model-Q4_K_M.gguf', bytes: 4, sha256: 'b' },
      { filename: 'mmproj-f16.gguf', bytes: 1, sha256: 'c' },
    ])).toBe('model-Q4_K_M.gguf');
  });

  it('labels GPU vs RAM fit from file size', () => {
    const machine = hardware(4096, 16 * 2 ** 30);
    expect(fitKind(2 * 2 ** 30, machine)).toBe('gpu');
    expect(fitKind(8 * 2 ** 30, machine)).toBe('ram');
    expect(fitKind(14 * 2 ** 30, machine)).toBe('large');
    expect(repoParts('bartowski/Qwen2.5-7B-Instruct-GGUF')).toEqual({
      publisher: 'bartowski', name: 'Qwen2.5 7B Instruct', id: 'bartowski/Qwen2.5-7B-Instruct-GGUF',
    });
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
