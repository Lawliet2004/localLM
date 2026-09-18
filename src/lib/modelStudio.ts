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
  const match = name.match(/(IQ\d+_[A-Z]+|Q\d+_K_[MSx]|Q\d+_K|Q\d+_[01]|[QF]16|F32|BF16)(?=[.\-_']|$)/i);
  return match ? match[1].toUpperCase() : 'GGUF';
}

export function weightFiles(files: HubFile[]) {
  return files.filter(file => !file.filename.toLowerCase().includes('mmproj') && (!file.filename.includes('-of-') || file.filename.includes('-00001-of-')));
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

export function fitKind(bytes: number, hardware: HardwareStatus | null): FitKind {
  if (!bytes) return 'unknown';
  const gpu = hardware?.gpus[0];
  const vram = gpu?.memoryTotalMib != null ? gpu.memoryTotalMib * 2 ** 20 : null;
  const ram = hardware?.memoryTotalBytes ?? null;
  if (vram != null && bytes <= vram * 0.85) return 'gpu';
  if (ram != null && bytes <= ram * 0.55) return 'ram';
  if (vram == null && ram == null) return 'unknown';
  return 'large';
}

export function fitLabel(kind: FitKind) {
  if (kind === 'gpu') return 'Fits GPU';
  if (kind === 'ram') return 'Fits RAM';
  if (kind === 'large') return 'May not fit';
  return 'Size unknown';
}

export function compactCount(value: number) {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 10_000 ? 0 : 1)}k`;
  return value.toLocaleString();
}
