import { useState } from 'react';
import { api, errorMessage, type ModelInstallStatus } from '../lib/api';
import { dismissJob, isJobDismissed, refreshDownloadJobs, useDownloadJobs } from '../lib/downloadJobs';
import { JobProgress } from './JobProgress';

// Lives outside the settings pages so navigating back to chat keeps progress visible.

const TERMINAL = new Set(['failed', 'cancelled', 'interrupted', 'ready']);

function visible(job: ModelInstallStatus) {
  if (job.busy) return true;
  if (!TERMINAL.has(job.phase) || isJobDismissed(job)) return false;
  // A recovered ready row belongs to an earlier session; only in-session jobs show.
  return job.phase !== 'ready' || job.jobId !== 0;
}

function resumable(job: ModelInstallStatus) {
  return ['failed', 'cancelled', 'interrupted'].includes(job.phase);
}

async function resume(job: ModelInstallStatus) {
  if (job.key === 'runtime:standard') await api.installRuntime();
  else if (job.key.startsWith('model:catalog:')) await api.installModel(job.key.slice('model:catalog:'.length));
}

async function pause(job: ModelInstallStatus) {
  if (job.key === 'runtime:standard') await api.cancelRuntimeInstall(job.jobId);
  else await api.cancelModelInstall(job.jobId);
}

function JobRow({ job, rate, waiting }: { job: ModelInstallStatus; rate: number; waiting: boolean }) {
  const [acting, setActing] = useState(false);
  const [rowError, setRowError] = useState('');
  async function run(action: () => Promise<void>) {
    setActing(true); setRowError('');
    try { await action(); } catch (e) { setRowError(errorMessage(e)); }
    finally { setActing(false); refreshDownloadJobs(); }
  }
  return <section className="download-activity" aria-label={job.label || 'Download'}>
    <JobProgress job={job} rate={rate} waiting={waiting} actions={<>
      {job.busy && <button type="button" className="secondary" disabled={acting}
        onClick={() => void run(() => pause(job))}>{acting ? 'Pausing…' : 'Pause'}</button>}
      {!job.busy && resumable(job) && (job.key.startsWith('model:hf:')
        ? <span>Resume in My Models — the download may need your Hugging Face token.</span>
        : <button type="button" className="secondary" disabled={acting}
            onClick={() => void run(() => resume(job))}>Resume</button>)}
      {!job.busy && <button type="button" className="secondary" onClick={() => dismissJob(job)}>Dismiss</button>}
      {rowError && <span role="alert">{rowError}</span>}
    </>}>
      {job.busy && <small>You can keep chatting with a loaded model.</small>}
    </JobProgress>
  </section>;
}

export function DownloadActivity() {
  const { jobs, stale, rate, waiting } = useDownloadJobs();
  const rows = jobs.filter(visible);
  return <>
    {stale && rows.length > 0 && <p className="download-activity-note" role="status">Status may be out of date.</p>}
    {rows.map(job => <JobRow key={`${job.key}:${job.jobId}`} job={job} rate={rate(job.jobId)} waiting={waiting(job.jobId)} />)}
  </>;
}
