import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Trajectory } from './Trajectory';
import type { SessionEvent } from '../lib/types';

const apiMocks = vi.hoisted(() => ({
  getSessionEvents: vi.fn(),
  forkSession: vi.fn(),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    getSessionEvents: apiMocks.getSessionEvents,
    forkSession: apiMocks.forkSession,
  },
}));

function makeEvents(conversationId: string, fromSeq: number, count: number): SessionEvent[] {
  return Array.from({ length: count }, (_, i) => {
    const seq = fromSeq + i;
    return {
      id: `${conversationId}-ev-${seq}`,
      conversationId,
      runId: 'r1',
      seq,
      stepId: null,
      toolCallId: null,
      eventType: 'user_msg',
      payload: `${conversationId} private content ${seq}`,
      ignorable: false,
      createdAt: seq,
    };
  });
}

describe('trajectory viewer', () => {
  it('does not navigate to an older fork after the user switches conversations', async () => {
    let resolveFork!: (value: unknown) => void;
    apiMocks.getSessionEvents.mockImplementation((id: string) => Promise.resolve(makeEvents(id, 1, 1)));
    apiMocks.forkSession.mockImplementationOnce(() => new Promise(resolve => { resolveFork = resolve; }));
    const onFork = vi.fn();
    const { rerender } = render(<Trajectory conversationId="A" onFork={onFork} />);
    fireEvent.click(screen.getByText(/Trajectory/));
    fireEvent.click(await screen.findByRole('button', { name: /Fork from here/ }));
    rerender(<Trajectory conversationId="B" onFork={onFork} />);
    await screen.findByText('B private content 1');
    await act(async () => { resolveFork({ newConversation: { id: 'old-fork' } }); });
    expect(onFork).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: /Fork from here/ })).toBeEnabled();
  });
  beforeEach(() => {
    apiMocks.getSessionEvents.mockReset();
    apiMocks.forkSession.mockReset();
  });

  const sampleEvents: SessionEvent[] = [
    {
      id: 'ev-1',
      conversationId: 'c1',
      runId: 'r1',
      seq: 1,
      stepId: 'turn_start',
      toolCallId: null,
      eventType: 'user_msg',
      payload: { role: 'user', content: 'What time is it?' },
      ignorable: false,
      createdAt: 1000,
    },
    {
      id: 'ev-2',
      conversationId: 'c1',
      runId: 'r1',
      seq: 2,
      stepId: 'step-0',
      toolCallId: 'call-1',
      eventType: 'tool_call',
      payload: { callId: 'call-1', name: 'system_time', arguments: {} },
      ignorable: false,
      createdAt: 2000,
    },
    {
      id: 'ev-3',
      conversationId: 'c1',
      runId: 'r1',
      seq: 3,
      stepId: 'step-0',
      toolCallId: 'call-1',
      eventType: 'tool_result',
      payload: { callId: 'call-1', name: 'system_time', result: { time: '12:00' } },
      ignorable: false,
      createdAt: 3000,
    },
    {
      id: 'ev-4',
      conversationId: 'c1',
      runId: 'r1',
      seq: 4,
      stepId: 'step-0',
      toolCallId: null,
      eventType: 'reasoning',
      payload: { text: 'I should format the time nicely.' },
      ignorable: true,
      createdAt: 4000,
    },
    {
      id: 'ev-5',
      conversationId: 'c1',
      runId: 'r1',
      seq: 5,
      stepId: 'step-0',
      toolCallId: null,
      eventType: 'step_end',
      payload: { round: 0, content: 'It is 12:00 PM.' },
      ignorable: false,
      createdAt: 5000,
    },
    {
      id: 'ev-6',
      conversationId: 'c1',
      runId: 'r1',
      seq: 6,
      stepId: null,
      toolCallId: null,
      eventType: 'error',
      payload: { error: 'Network failure' },
      ignorable: false,
      createdAt: 6000,
    },
  ];

  it('loads session events and displays event badges and sequence numbers', async () => {
    apiMocks.getSessionEvents.mockResolvedValue(sampleEvents);
    render(<Trajectory conversationId="c1" />);
    fireEvent.click(screen.getByText(/Trajectory/));

    await screen.findByText('#1');
    expect(screen.getByText('user_msg')).toBeInTheDocument();
    expect(screen.getByText('#2')).toBeInTheDocument();
    expect(screen.getByText('tool_call')).toBeInTheDocument();
    expect(screen.getByText('#3')).toBeInTheDocument();
    expect(screen.getByText('tool_result')).toBeInTheDocument();
  });

  it('filters events by category tabs', async () => {
    apiMocks.getSessionEvents.mockResolvedValue(sampleEvents);
    render(<Trajectory conversationId="c1" />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await screen.findByText('#1');

    // Click "Tool" tab
    fireEvent.click(screen.getByRole('button', { name: 'Tool' }));
    expect(screen.queryByText('user_msg')).not.toBeInTheDocument();
    expect(screen.getByText('tool_call')).toBeInTheDocument();
    expect(screen.getByText('tool_result')).toBeInTheDocument();
    expect(screen.queryByText('reasoning')).not.toBeInTheDocument();

    // Click "Model" tab
    fireEvent.click(screen.getByRole('button', { name: 'Model' }));
    expect(screen.queryByText('tool_call')).not.toBeInTheDocument();
    expect(screen.getByText('reasoning')).toBeInTheDocument();
    expect(screen.getByText('step_end')).toBeInTheDocument();

    // Click "Error" tab
    fireEvent.click(screen.getByRole('button', { name: 'Error' }));
    expect(screen.getByText('error')).toBeInTheDocument();
    expect(screen.queryByText('user_msg')).not.toBeInTheDocument();

    // Click "System" tab
    fireEvent.click(screen.getByRole('button', { name: 'System' }));
    expect(screen.getByText('user_msg')).toBeInTheDocument();
  });

  it('filters events using search box', async () => {
    apiMocks.getSessionEvents.mockResolvedValue(sampleEvents);
    render(<Trajectory conversationId="c1" />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await screen.findByText('#1');

    fireEvent.change(screen.getByLabelText('Filter trajectory events'), {
      target: { value: 'system_time' },
    });
    expect(screen.getByText('tool_call')).toBeInTheDocument();
    expect(screen.getByText('tool_result')).toBeInTheDocument();
    expect(screen.queryByText('user_msg')).not.toBeInTheDocument();
  });

  it('invokes forkSession and calls onFork callback', async () => {
    apiMocks.getSessionEvents.mockResolvedValue(sampleEvents);
    apiMocks.forkSession.mockResolvedValue({
      newConversation: { id: 'forked-conv-1', title: 'Fork of c1' },
      copiedEventsCount: 2,
    });
    const onFork = vi.fn();

    render(<Trajectory conversationId="c1" onFork={onFork} />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await screen.findByText('#1');

    const forkButtons = screen.getAllByRole('button', { name: /Fork from here/i });
    expect(forkButtons.length).toBeGreaterThan(0);

    fireEvent.click(forkButtons[1]); // Fork at event #2
    expect(apiMocks.forkSession).toHaveBeenCalledWith('c1', 2);

    // Wait for callback
    await vi.waitFor(() => {
      expect(onFork).toHaveBeenCalledWith('forked-conv-1');
    });
  });

  it('shows appropriate empty states', () => {
    render(<Trajectory conversationId={null} />);
    fireEvent.click(screen.getByText(/Trajectory/));
    expect(screen.getByText('Start a conversation to see its run trajectory.')).toBeInTheDocument();
  });

  it('drops a stale response when the conversation changes mid-load (A08)', async () => {
    let resolveA: (events: SessionEvent[]) => void = () => {};
    apiMocks.getSessionEvents.mockImplementation((id: string) =>
      id === 'conversation-A'
        ? new Promise<SessionEvent[]>(resolve => { resolveA = resolve; })
        : Promise.resolve(makeEvents('conversation-B', 1, 1)));

    const { rerender } = render(<Trajectory conversationId="conversation-A" />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await waitFor(() => expect(apiMocks.getSessionEvents).toHaveBeenCalledWith('conversation-A', null, 500));

    rerender(<Trajectory conversationId="conversation-B" />);
    // B's load is not blocked by A's in-flight request.
    await screen.findByText('conversation-B private content 1');
    expect(apiMocks.getSessionEvents).toHaveBeenCalledWith('conversation-B', null, 500);

    // A's late response must not write rows, errors, or loading state for B.
    await act(async () => { resolveA(makeEvents('conversation-A', 900, 1)); });
    expect(screen.queryByText('conversation-A private content 900')).not.toBeInTheDocument();
    expect(screen.queryByText('#900')).not.toBeInTheDocument();
    expect(screen.getByText('conversation-B private content 1')).toBeInTheDocument();
  });

  it('does not surface a stale error from a previous conversation (A08)', async () => {
    let rejectA: (e: Error) => void = () => {};
    apiMocks.getSessionEvents.mockImplementation((id: string) =>
      id === 'conversation-A'
        ? new Promise<SessionEvent[]>((_, reject) => { rejectA = reject; })
        : Promise.resolve(makeEvents('conversation-B', 1, 1)));

    const { rerender } = render(<Trajectory conversationId="conversation-A" />);
    fireEvent.click(screen.getByText(/Trajectory/));
    rerender(<Trajectory conversationId="conversation-B" />);
    await screen.findByText('conversation-B private content 1');

    await act(async () => { rejectA(new Error('conversation-A failed')); });
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('conversation-B private content 1')).toBeInTheDocument();
  });

  it('forks the conversation that owns the rendered event after switching (A08)', async () => {
    apiMocks.getSessionEvents.mockImplementation((id: string) => Promise.resolve(makeEvents(id, 1, 2)));
    apiMocks.forkSession.mockResolvedValue({
      newConversation: { id: 'forked-conv', title: 'Fork', updatedAt: 0, providerId: null, modelId: null, providerSelectionRequired: false },
      copiedEventsCount: 1,
    });
    const onFork = vi.fn();

    const { rerender } = render(<Trajectory conversationId="conversation-A" onFork={onFork} />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await screen.findByText('conversation-A private content 1');

    rerender(<Trajectory conversationId="conversation-B" onFork={onFork} />);
    await screen.findByText('conversation-B private content 1');
    expect(screen.queryByText('conversation-A private content 1')).not.toBeInTheDocument();

    const forkButtons = screen.getAllByRole('button', { name: /Fork from here/i });
    expect(forkButtons.length).toBe(2);
    fireEvent.click(forkButtons[0]);
    expect(apiMocks.forkSession).toHaveBeenCalledWith('conversation-B', 1);
    await waitFor(() => expect(onFork).toHaveBeenCalledWith('forked-conv'));
  });

});
