// LocalLM TypeScript SDK (Phase 7, sdk-minimal profile).
// Typed wrappers over the Tauri invoke surface for WebView/test use, plus a
// loopback HTTP client mirroring python/locallm_sdk.py (needs the webhook
// listener enabled + a rotated token).

import { invoke } from '@tauri-apps/api/core';
import type { Capability } from './types';

export interface PresetSummary { id: string; name: string; description: string }
export interface ScheduleSummary {
  id: string; name: string; cron: string; task: string;
  enabled: boolean; runOnce: boolean;
  lastRunAt: number | null; lastResult: string | null;
}

export const sdk = {
  listCapabilities: (conversationId?: string | null) =>
    invoke<Capability[]>('list_capabilities', { conversationId }),
  dumpConfig: (conversationId?: string | null) =>
    invoke<Record<string, unknown>>('dump_config', { conversationId }),
  listPresets: () => invoke<PresetSummary[]>('list_presets'),
  setPreset: (conversationId: string, preset: string) =>
    invoke<PresetSummary>('set_preset', { conversationId, preset }),
  listSchedules: () => invoke<ScheduleSummary[]>('list_schedules'),
  runScheduleNow: (id: string) => invoke<string>('run_schedule_now', { id }),
  interruptSubagent: (childRunId: string) => invoke<boolean>('interrupt_subagent', { childRunId }),
};

export interface LoopbackOptions { host?: string; port?: number; token: string }

async function loopback<T>(options: LoopbackOptions, method: string, path: string, body?: unknown): Promise<T> {
  const response = await fetch(`http://${options.host ?? '127.0.0.1'}:${options.port ?? 4317}${path}`, {
    method,
    headers: { Authorization: `Bearer ${options.token}`, 'Content-Type': 'application/json' },
    body: body === undefined && method === 'GET' ? undefined : JSON.stringify(body ?? {}),
  });
  if (!response.ok) throw new Error(`LocalLM API ${response.status}: ${await response.text()}`);
  return (await response.json()) as T;
}

export const loopbackSdk = {
  config: (options: LoopbackOptions) => loopback<Record<string, unknown>>(options, 'GET', '/api/config'),
  schedules: (options: LoopbackOptions) => loopback<ScheduleSummary[]>(options, 'GET', '/api/schedules'),
  runSchedule: (options: LoopbackOptions, id: string) =>
    loopback<{ accepted: string }>(options, 'POST', '/api/schedules/run', { id }),
  webhook: (options: LoopbackOptions, id: string) =>
    loopback<{ accepted: string }>(options, 'POST', `/webhook/${id}`, {}),
};
