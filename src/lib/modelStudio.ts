import { freshHardware } from './runtimeGuidance';
import type { HardwareStatus, HubFile } from './types';

export function parseHubQuery(query: string) {
  const trimmed = query.trim().replace(/[\\]/g, '/');
  const fromUrl = trimmed.match(/huggingface\.co\/+([^/?#]+\/[^/?#]+)/i);
  if (fromUrl) return fromUrl[1].replace(/\/+$/, '');
  if (/^[^/\s]+\/[^/\s]+$/.test(trimmed)) return trimmed.replace(/\/+$/, '');
  return null;
}

export function repoParts(repo: string) {
  const [publisher = '', ...rest] = repo.split('/');
  const slug = rest.join('/') || repo;
  return {
    publisher,
    name: slug.replace(/-GGUF$/i, '').replace(/-/g, ' '),
    id: repo,
  };
}

export function quantLabel(filename: string) {
  const name = filename.split(/[\\/]/).pop() ?? filename;
  const match = name.match(/(PQ\d+_[01]|PTQ\d+_[01]|IQ\d+_[A-Z]+|Q\d+_K_[MSx]|Q\d+_K|Q\d+_[01]|[QF]16|F32|BF16)(?=[.\-_']|$)/i);
  return match ? match[1].toUpperCase() : 'GGUF';
}

export function weightFiles(files: HubFile[]) {
  return files.filter(file => isWeightFile(file.filename) && (!file.filename.includes('-of-') || file.filename.includes('-00001-of-')));
}

export function isWeightFile(filename: string) {
  const lower = filename.replace(/\\/g, '/').toLowerCase();
  // MTP heads and calibration/adapter files can also be GGUFs. A model that
  // embeds MTP may still have an MTP suffix, so only exclude companion prefixes
  // and directories, rather than every occurrence of "mtp".
  return lower.endsWith('.gguf') && !lower.includes('mmproj')
    && !/(^|\/)(mtp|imatrix|adapters?|lora|projectors?)(?=[/._-]|$)/.test(lower);
}

export function hubSize(bytes: number) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(2)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(2)} MB`;
  if (bytes >= 1e3) return `${(bytes / 1e3).toFixed(2)} kB`;
  return `${bytes} B`;
}

export function projectorFiles(files: HubFile[]) {
  return files
    .filter(file => file.filename.toLowerCase().endsWith('.gguf') && file.filename.toLowerCase().includes('mmproj'))
    .sort((a, b) => a.filename.localeCompare(b.filename));
}

// Prefer the projector that shares the selected weight file's quant stem
// (e.g. BAAI_AREX-Turbo-Q4_K_M ↔ mmproj-BAAI_AREX-Turbo-Q4_K_M); otherwise the
// repo's only projector; otherwise none so the user must choose explicitly.
export function preferredProjector(files: HubFile[], weightFilename: string): string {
  const candidates = projectorFiles(files);
  if (!candidates.length) return '';
  const stem = (weightFilename.split('/').pop() ?? weightFilename).replace(/\.gguf$/i, '').toLowerCase();
  const stemCore = stem.replace(/^mmproj[-_]?/, '');
  const match = candidates.find(file => {
    const base = (file.filename.split('/').pop() ?? file.filename).toLowerCase().replace(/\.gguf$/, '');
    const baseCore = base.replace(/^mmproj[-_]?/, '');
    return base.includes(stem) || stem.includes(baseCore) || baseCore.includes(stemCore);
  });
  if (match) return match.filename;
  return candidates.length === 1 ? candidates[0].filename : '';
}

export function downloadSize(files: HubFile[], filename: string) {
  const shard = filename.match(/^(.*)-\d{5}-of-\d{5}\.gguf$/);
  if (shard) return files.filter(file => file.filename.startsWith(`${shard[1]}-`) && /-\d{5}-of-\d{5}\.gguf$/.test(file.filename)).reduce((total, file) => total + file.bytes, 0);
  return files.find(file => file.filename === filename)?.bytes;
}

const preferredQuants = ['q4_k_m', 'q5_k_m', 'q6_k', 'q5_k_s', 'q4_k_s', 'q8_0', 'q4_0'];

export function preferredFile(files: HubFile[]) {
  const weights = weightFiles(files);
  for (const quant of preferredQuants) {
    const match = weights.find(file => file.filename.toLowerCase().includes(quant));
    if (match) return match.filename;
  }
  return weights[0]?.filename ?? '';
}

export type FitKind = 'gpu' | 'ram' | 'large' | 'unknown';

export function fitKind(bytes: number, hardware: HardwareStatus | null, projectorBytes = 0): FitKind {
  if (bytes <= 0 || !Number.isFinite(bytes) || projectorBytes < 0 || !Number.isFinite(projectorBytes)) return 'unknown';
  // Discovery has file sizes, not the architecture/context metadata needed to
  // predict the KV cache. Reserve runtime headroom and label results candidates.
  hardware = freshHardware(hardware);
  const weights = bytes + projectorBytes;
  const required = weights + Math.max(2 ** 30, weights * 0.1);
  // Candidate only: another runtime may select a different device.
  const gpu = hardware?.gpus.reduce((best, item) => {
    const free = (item.memoryTotalMib ?? 0) - (item.memoryUsedMib ?? 0);
    const previous = (best?.memoryTotalMib ?? 0) - (best?.memoryUsedMib ?? 0);
    return free > previous ? item : best;
  }, hardware.gpus[0]);
  const vram = gpu?.memoryTotalMib != null && gpu.memoryUsedMib != null
    ? Math.max(0, gpu.memoryTotalMib - gpu.memoryUsedMib) * 2 ** 20 : null;
  const ram = hardware?.memoryAvailableBytes ?? null;
  if (vram != null && required <= vram) return 'gpu';
  if (ram != null && required <= ram) return 'ram';
  if (vram == null && ram == null) return 'unknown';
  return 'large';
}

export function fitLabel(kind: FitKind) {
  if (kind === 'gpu') return 'GPU candidate';
  if (kind === 'ram') return 'RAM candidate';
  if (kind === 'large') return 'Memory tight';
  return 'Memory unknown';
}

export function compactCount(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return value.toLocaleString();
}
