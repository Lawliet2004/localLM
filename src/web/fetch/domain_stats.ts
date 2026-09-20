/**
 * Per-domain fetch learning: records live-fetch hit/fail outcomes per domain
 * and surfaces chronic failure so the fetch stage can stop burning its page
 * budget on sites that block automated retrieval (403 walls, bot challenges).
 * Modeled on a 30-day sliding window with read-time reset: stale stats count
 * as empty regardless of write cadence. Persisted through the shared storage
 * adapter (SQLite in the app, in-memory in tests); a storage failure never
 * surfaces to the fetch path.
 */

import type { StorageAdapter } from '../cache/sqlite';
import type { SearchResult } from '../types';

export interface DomainTierStat {
  attempts: number;
  ok: number;
  fail: number;
  windowStartMs: number;
  lastError?: string;
  lastAttemptAt?: string;
}

export interface DomainStatus {
  attempts: number;
  ok: number;
  fail: number;
  /** ok / attempts within the current window, or null when nothing was attempted yet. */
  failureRate: number | null;
  /** True when enough attempts accumulated to distrust live fetches for this domain. */
  chronic: boolean;
}

const DOMAIN_RECORD_TTL_SECONDS = 90 * 24 * 60 * 60;

export class DomainStatsStore {
  constructor(
    private storage: StorageAdapter,
    private options: { windowMs?: number; minAttempts?: number; chronicFailRate?: number } = {},
  ) {}

  private get windowMs(): number {
    return this.options.windowMs ?? 30 * 24 * 60 * 60 * 1000;
  }

  private get minAttempts(): number {
    return Math.max(2, this.options.minAttempts ?? 4);
  }

  private get chronicFailRate(): number {
    return this.options.chronicFailRate ?? 0.7;
  }

  /** Current-window view of a stored stat: an expired window reads as empty. */
  private currentWindow(stat: DomainTierStat | null | undefined): DomainTierStat {
    if (!stat || typeof stat.attempts !== 'number') {
      return { attempts: 0, ok: 0, fail: 0, windowStartMs: Date.now() };
    }
    if (Date.now() - stat.windowStartMs <= this.windowMs) return stat;
    return { attempts: 0, ok: 0, fail: 0, windowStartMs: stat.windowStartMs };
  }

  async record(domain: string, ok: boolean, error?: string): Promise<void> {
    const key = domain.toLowerCase();
    if (!key) return;
    try {
      const stored = await this.storage.get<DomainTierStat>('domain_stats', key);
      const stat = this.currentWindow(stored);
      stat.attempts += 1;
      if (ok) stat.ok += 1;
      else {
        stat.fail += 1;
        if (error) stat.lastError = String(error).slice(0, 200);
      }
      stat.lastAttemptAt = new Date().toISOString();
      await this.storage.set('domain_stats', key, stat, DOMAIN_RECORD_TTL_SECONDS);
    } catch {
      // Best-effort: learning must never break fetching.
    }
  }

  async status(domain: string): Promise<DomainStatus> {
    const key = domain.toLowerCase();
    try {
      const stat = this.currentWindow(await this.storage.get<DomainTierStat>('domain_stats', key));
      const failureRate = stat.attempts > 0 ? stat.fail / stat.attempts : null;
      return {
        attempts: stat.attempts,
        ok: stat.ok,
        fail: stat.fail,
        failureRate,
        chronic: stat.attempts >= this.minAttempts && failureRate !== null && failureRate >= this.chronicFailRate,
      };
    } catch {
      return { attempts: 0, ok: 0, fail: 0, failureRate: null, chronic: false };
    }
  }
}

/**
 * Keep chronic failing domains in the fetch list (Wayback may still recover
 * them) but move them behind hosts that have been working, so a small page
 * budget is spent on pages the live fetcher can actually read.
 */
export async function orderByDomainHealth(
  results: SearchResult[],
  stats: DomainStatsStore,
): Promise<{ ordered: SearchResult[]; deprioritized: string[] }> {
  if (results.length <= 1) return { ordered: results, deprioritized: [] };
  const healthy: SearchResult[] = [];
  const chronic: SearchResult[] = [];
  const deprioritized: string[] = [];
  for (const result of results) {
    const status = await stats.status(result.domain);
    if (status.chronic) {
      chronic.push(result);
      if (!deprioritized.includes(result.domain)) deprioritized.push(result.domain);
    } else {
      healthy.push(result);
    }
  }
  return { ordered: [...healthy, ...chronic], deprioritized };
}
