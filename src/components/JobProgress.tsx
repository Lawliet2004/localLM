import type { ReactNode } from 'react';
import type { ModelInstallStatus } from '../lib/api';
import { gib } from '../lib/runtimeGuidance';

// Shared presentation for one acquisition job. The banner and the settings
// panels render this so phase wording and progress semantics stay identical.

function title(job: ModelInstallStatus) {
  const kind = job.key.startsWith('runtime:') ? 'archive' : 'file';
  const count = job.assetCount > 1 ? ` ${job.assetIndex} of ${job.assetCount}` : '';
  switch (job.phase) {
    case 'preparing':
    case 'resolving': return 'Preparing';
    case 'downloading':
    case 'resuming': return `Downloading${job.assetCount > 1 ? ` ${kind}${count}` : ''}`;
    case 'retrying': return `Retrying (attempt ${job.attempt || 1})`;
    case 'verifying': return job.asset ? `Verifying ${job.asset}${count ? ` (${count.trim()})` : ''}` : 'Verifying';
    case 'extracting': return job.asset ? `Extracting ${job.asset}${count ? ` (${count.trim()})` : ''}` : 'Extracting';
    case 'finalizing': return 'Finalizing installation';
    case 'ready': return job.key.startsWith('runtime:') ? 'Installed' : 'Downloaded and verified';
    case 'cancelled': return 'Paused';
    case 'interrupted': return 'Interrupted';
    case 'failed': return 'Failed';
    default: return job.label || 'Working';
  }
}

export function JobProgress({ job, rate = 0, waiting = false, actions, children }: {
  job: ModelInstallStatus;
  rate?: number;
  waiting?: boolean;
  actions?: ReactNode;
  children?: ReactNode;
}) {
  const transferring = job.phase === 'downloading' || job.phase === 'resuming';
  const terminal = !job.busy;
  const determinate =
    transferring || job.phase === 'retrying' || job.phase === 'verifying' ||
    (terminal && job.total > 0);
  const eta = transferring && rate > 0 && job.total > job.received
    ? ` · ${(rate / 1048576).toFixed(1)} MiB/s · ~${Math.max(1, Math.ceil((job.total - job.received) / rate / 60))} min left`
    : '';
  return <div className="download-activity-copy">
    <strong role="status">{title(job)}</strong>
    {job.asset && <small title={job.asset}>{job.asset}</small>}
    <span>{gib(job.received)} / {job.total ? gib(job.total) : 'checking size'}
      {job.saved > 0 ? ` · ${gib(job.saved)} were already saved` : ''}{eta}
      {job.busy && waiting ? ' · Waiting for progress…' : ''}</span>
    {job.phase === 'verifying'
      ? <progress aria-label="Verification progress" max={job.assetTotal || 1} value={job.assetReceived} />
      : determinate
        ? <progress aria-label={`${job.label || 'Download'} progress`} max={job.total || 1} value={job.received} />
        : job.busy ? <progress aria-label={`${job.label || 'Download'} progress`} /> : null}
    {job.notice && <span>{job.notice}</span>}
    {job.error && (job.phase === 'cancelled'
      ? <span role="status">Paused. Saved bytes are kept; Resume continues from them.</span>
      : <span role="alert">{job.error}</span>)}
    {children}
    {actions && <div className="connector-actions">{actions}</div>}
  </div>;
}
