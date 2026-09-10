import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Trajectory } from './Trajectory';
import type { RunEvent } from '../lib/types';

const apiMocks = vi.hoisted(() => ({
  getConversationRuns: vi.fn(),
  getRunEvents: vi.fn(),
  listSubagentRuns: vi.fn(),
  interruptSubagent: vi.fn(),
  compactConversation: vi.fn(),
}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    getConversationRuns: apiMocks.getConversationRuns,
    getRunEvents: apiMocks.getRunEvents,
    listSubagentRuns: apiMocks.listSubagentRuns,
    interruptSubagent: apiMocks.interruptSubagent,
    compactConversation: apiMocks.compactConversation,
  },
}));

describe('trajectory viewer', () => {
  it('loads runs plus the subagent tree and filters events by payload', async () => {
    apiMocks.getConversationRuns.mockResolvedValue([
      { id: 'run-1', conversationId: 'c1', status: 'completed', createdAt: 0, updatedAt: 0 },
    ]);
    apiMocks.listSubagentRuns.mockResolvedValue([
      { id: 'sub-1', conversationId: 'c1', parentRunId: 'run-1', childRunId: 'child-1', depth: 1, status: 'completed', label: 'research', prompt: 'go', error: null, createdAt: 0, updatedAt: 0 },
    ]);
    const events: RunEvent[] = [
      { runId: 'run-1', seq: 0, stepId: 's1', eventType: 'tool_call', payload: { name: 'system_time' }, createdAt: 0 },
      { runId: 'run-1', seq: 1, stepId: 's1', eventType: 'tool_result', payload: { ok: true }, createdAt: 0 },
    ];
    apiMocks.getRunEvents.mockResolvedValue(events);
    render(<Trajectory conversationId="c1" />);
    fireEvent.click(screen.getByText(/Trajectory/));
    await screen.findByText(/#0 · tool_call/);
    expect(screen.getByText(/research/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('Filter trajectory events'), { target: { value: 'system_time' } });
    expect(screen.getByText(/#0 · tool_call/)).toBeInTheDocument();
    expect(screen.queryByText(/#1 · tool_result/)).not.toBeInTheDocument();
  });
  it('explains itself when there is no conversation yet', () => {
    render(<Trajectory conversationId={null} />);
    fireEvent.click(screen.getByText('Trajectory'));
    expect(screen.getByText('Start a conversation to see its run trajectory.')).toBeInTheDocument();
  });
});
