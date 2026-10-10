import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Evidence } from './Evidence';
import type { VerificationRecord, VerificationSummary } from '../lib/types';

const record = (partial: Partial<VerificationRecord>): VerificationRecord => ({
  id: 'vr1', conversationId: 'c1', criterionId: null, kind: 'command', method: 'shell command (powershell)',
  command: 'npm test', environment: 'native-local', startedAt: 1, endedAt: 2, exitCode: 0,
  outputExcerpt: 'all green', outputArtifact: 'art_1', outputTruncated: false, artifacts: [],
  result: 'passed', limitations: '', fingerprint: 'tree:abc', createdAt: 2, stale: false, ...partial,
});

const summary = (partial: Partial<VerificationSummary>): VerificationSummary => ({
  criteria: [], records: [], state: { state: 'work-completed', summary: 'Work completed — nothing has been verified.' }, ...partial,
});

const mocks = vi.hoisted(() => ({
  summary: vi.fn(async () => summary({}) as VerificationSummary),
  upsert: vi.fn(async () => ({})),
  delete: vi.fn(async () => {}),
  runUser: vi.fn(async () => ({})),
  output: vi.fn(async () => ({ text: 'FULL LOG BODY', truncated: false })),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => String(e),
  api: {
    verificationSummary: mocks.summary,
    criteriaUpsert: mocks.upsert,
    criteriaDelete: mocks.delete,
    verificationRunUser: mocks.runUser,
    verificationRecordOutput: mocks.output,
  },
}));

function renderEvidence() {
  return render(<Evidence conversation="c1" tick={1} />);
}

describe('evidence block', () => {
  it('shows the honest unverified badge when no checks ran', async () => {
    renderEvidence();
    expect(await screen.findByText('Work completed · not verified')).toBeInTheDocument();
  });

  it('maps each completion state to its badge', async () => {
    const cases: [string, string][] = [
      ['checks-passed', 'Checks passed'],
      ['checks-failed', 'Checks failed'],
      ['partially-verified', 'Partially verified'],
      ['needs-review', 'Needs review'],
    ];
    for (const [state, label] of cases) {
      mocks.summary.mockResolvedValueOnce(summary({ state: { state: state as never, summary: 'x' } }));
      const { unmount } = renderEvidence();
      expect(await screen.findByText(label)).toBeInTheDocument();
      unmount();
    }
  });

  it('expands to criteria, records, stale tags, and a full-log view', async () => {
    mocks.summary.mockResolvedValueOnce(summary({
      criteria: [{ id: 'cr1', conversationId: 'c1', text: 'tests pass', origin: 'agent', createdAt: 1, updatedAt: 1 }],
      records: [record({ stale: true, result: 'failed', exitCode: 1 })],
      state: { state: 'checks-failed', summary: '1 check failed.' },
    }));
    renderEvidence();
    fireEvent.click(await screen.findByRole('button', { name: /Checks failed/ }));
    expect(screen.getByText('tests pass', { selector: '.evidence-criterion-text' })).toBeInTheDocument();
    expect(screen.getByText('agent')).toBeInTheDocument();
    expect(screen.getByText('stale')).toBeInTheDocument();
    expect(screen.getByText('exit 1')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Show full log/ }));
    await waitFor(() => expect(mocks.output).toHaveBeenCalledWith('vr1'));
    expect(await screen.findByLabelText('Full check log')).toHaveTextContent('FULL LOG BODY');
  });

  it('runs a user check and adds criteria', async () => {
    renderEvidence();
    fireEvent.click(await screen.findByRole('button', { name: /not verified/ }));
    fireEvent.change(screen.getByLabelText('Check command'), { target: { value: 'cargo test' } });
    fireEvent.click(screen.getByRole('button', { name: /Run check/ }));
    await waitFor(() => expect(mocks.runUser).toHaveBeenCalledWith('c1', 'cargo test', undefined));
    fireEvent.change(screen.getByLabelText('New criterion'), { target: { value: 'lint clean' } });
    fireEvent.click(screen.getByRole('button', { name: /Add/ }));
    await waitFor(() => expect(mocks.upsert).toHaveBeenCalledWith('c1', null, 'lint clean'));
  });
});
