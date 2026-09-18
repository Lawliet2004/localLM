import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
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

describe('trajectory viewer', () => {
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
});
