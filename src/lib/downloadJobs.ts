import { useSyncExternalStore } from 'react';
import { api, errorMessage, nativeAvailable, type ModelInstallStatus } from './api';

// One status source for every acquisition job. The store lives at module level
// so the banner and every settings panel share one poll and survive navigation.

interface JobsState {
  jobs: ModelInstallStatus[];
  stale: boolean;
  error: string;
}

const TRANSFER_PHASES = new Set(['downloading', 'resuming']);
const POLL_MS = 1000;
const WAIT_MS = 10_000;

interface Sample { jobId: number; asset: string | null; received: number; time: number; rate: number; changed: number }

let state: JobsState = { jobs: [], stale: false, error: '' };
const listeners = new Set<() => void>();
const samples = new Map<number, Sample>();
const dismissed = new Set<string>();
let timer: ReturnType<typeof setInterval> | undefined;
let polling = false;

function emit(next: JobsState) {
  state = next;
  listeners.forEach(listener => listener());
}

function measure(jobs: ModelInstallStatus[]) {
  const now = Date.now();
  for (const job of jobs) {
    const previous = samples.get(job.jobId);
    const sample: Sample = {
      jobId: job.jobId, asset: job.asset, received: job.received, time: now,
      rate: 0, changed: previous?.changed ?? now,
    };
    if (TRANSFER_PHASES.has(job.phase) && previous && previous.asset === job.asset) {
      const elapsed = (now - previous.time) / 1000;
      const delta = job.received - previous.received;
      sample.rate = delta > 0 && elapsed > 0 ? delta / elapsed : 0;
      if (delta > 0) sample.changed = now;
    }
    samples.set(job.jobId, sample);
  }
}

async function poll() {
  if (polling || !listeners.size || !nativeAvailable) return;
  polling = true;
  try {
    const jobs = await api.downloadJobs();
    measure(jobs);
    emit({ jobs, stale: false, error: '' });
  } catch (cause) {
    // Keep the last good list — a failed poll never clears visible jobs.
    emit({ jobs: state.jobs, stale: true, error: errorMessage(cause) });
  } finally {
    polling = false;
  }
}

export function refreshDownloadJobs() {
  void poll();
}

export function dismissJob(job: ModelInstallStatus) {
  dismissed.add(`${job.key}:${job.jobId}:${job.phase}`);
  emit({ ...state });
}

export function isJobDismissed(job: ModelInstallStatus) {
  return dismissed.has(`${job.key}:${job.jobId}:${job.phase}`);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (listeners.size === 1 && nativeAvailable) {
    void poll();
    timer = setInterval(() => void poll(), POLL_MS);
  }
  return () => {
    listeners.delete(listener);
    if (!listeners.size && timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  };
}

export interface DownloadJobs {
  jobs: ModelInstallStatus[];
  stale: boolean;
  error: string;
  /** Bytes/sec for a job's current transfer, 0 outside download phases. */
  rate: (jobId: number) => number;
  /** Busy job with no byte change for at least 10 seconds. */
  waiting: (jobId: number) => boolean;
}

export function useDownloadJobs(): DownloadJobs {
  const snapshot = useSyncExternalStore(subscribe, () => state, () => state);
  return {
    jobs: snapshot.jobs,
    stale: snapshot.stale,
    error: snapshot.error,
    rate: jobId => samples.get(jobId)?.rate ?? 0,
    waiting: jobId => {
      const job = snapshot.jobs.find(candidate => candidate.jobId === jobId);
      const sample = samples.get(jobId);
      return Boolean(job?.busy && sample && Date.now() - sample.changed >= WAIT_MS);
    },
  };
}

export function useDownloadJob(key: string): ModelInstallStatus | undefined {
  const snapshot = useSyncExternalStore(subscribe, () => state, () => state);
  return snapshot.jobs.find(job => job.key === key);
}
