import type { ModelDownloadInfo } from '../../lib/api';
import type { RuntimeConfig } from '../../lib/types';

export type BonsaiAsset = 'model' | 'runtime';
export interface BonsaiInfo {
  catalog: { label: string; experimental: boolean; chatOnly: boolean };
  model: ModelDownloadInfo;
  runtime: { bytes: number; requiredBytes: number; availableBytes: number; destination: string };
  runtimePath: string | null;
  preset: RuntimeConfig | null;
}
