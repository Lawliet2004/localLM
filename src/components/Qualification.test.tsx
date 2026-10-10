import { describe, expect, it, vi, beforeEach } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { Qualification, Readiness } from '../lib/types';

const apiMocks = vi.hoisted(() => ({
  readiness: vi.fn<() => Promise<Readiness>>(async () => ({ step: 'ready', message: 'ok', nextAction: null })),
  qualificationResults: vi.fn<() => Promise<Qualification | null>>(async () => null),
  qualificationRun: vi.fn(async () => {}),
  qualificationCancel: vi.fn(async () => {}),
}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: apiMocks,
}));

import { ReadinessCard, QualificationPanel } from './Qualification';

const baseCase = {
  name: 'basic_chat', status: 'passed' as const, latencyMs: 120, tokensPerSecond: 42.5,
  error: null, detail: null,
};

const record: Qualification = {
  id: 'q1', identityHash: 'h1', stale: false, modelId: 'm.gguf', modelPath: 'm.gguf',
  providerId: null, runtimeVersion: 'local', config: {}, startedAt: 1, endedAt: 2,
  cases: [
    baseCase,
    { ...baseCase, name: 'tool_call', status: 'failed', latencyMs: 90, tokensPerSecond: null, error: 'No tool call emitted; reply was: I cannot' },
    { ...baseCase, name: 'grounded_research', status: 'untested', tokensPerSecond: null, detail: 'Web search is not configured' },
  ],
  status: 'partial',
};

describe('readiness card', () => {
  beforeEach(() => vi.clearAllMocks());

  it('shows the missing step and its next action', async () => {
    apiMocks.readiness.mockResolvedValueOnce({
      step: 'model-missing', message: 'No model is selected.',
      nextAction: { label: 'Choose a model in Models', target: 'models' },
    });
    const onAction = vi.fn();
    render(<ReadinessCard onAction={onAction} />);
    await screen.findByText('No model is selected.');
    expect(screen.getByText('Choose a model')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Choose a model in Models' }));
    expect(onAction).toHaveBeenCalledWith('models');
  });

  it('renders nothing when ready', async () => {
    apiMocks.readiness.mockResolvedValueOnce({ step: 'ready', message: 'Model is loaded and qualified.', nextAction: null });
    const { container } = render(<ReadinessCard onAction={() => {}} />);
    await waitFor(() => expect(apiMocks.readiness).toHaveBeenCalled());
    expect(container.firstChild).toBeNull();
  });
});

describe('qualification panel', () => {
  beforeEach(() => vi.clearAllMocks());

  it('shows per-case results with untested distinct from failures', async () => {
    apiMocks.qualificationResults.mockResolvedValueOnce(record);
    render(<QualificationPanel selection={{ providerId: null, modelId: '' }} busy={false} />);
    await screen.findByText('tool call');
    expect(screen.getByText('partial')).toBeInTheDocument();
    expect(screen.getByText('failed')).toBeInTheDocument();
    expect(screen.getByText('untested')).toBeInTheDocument();
    expect(screen.getByText('42.5 tok/s')).toBeInTheDocument();
    expect(screen.getByText(/No tool call emitted/)).toBeInTheDocument();
  });

  it('marks a record stale when the identity changed', async () => {
    apiMocks.qualificationResults.mockResolvedValueOnce({ ...record, stale: true });
    render(<QualificationPanel selection={{ providerId: null, modelId: '' }} busy={false} />);
    await screen.findByText(/no longer describes the current model/);
  });

  it('runs the suite over the current selection', async () => {
    apiMocks.qualificationResults.mockResolvedValue(null);
    render(<QualificationPanel selection={{ providerId: null, modelId: '' }} busy={false} />);
    await screen.findByRole('button', { name: /Run qualification/ });
    fireEvent.click(screen.getByRole('button', { name: /Run qualification/ }));
    await waitFor(() => expect(apiMocks.qualificationRun).toHaveBeenCalledWith(
      { providerId: null, modelId: '' }, expect.anything(),
    ));
  });
});
