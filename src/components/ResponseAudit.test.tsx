import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ActivityTimeline, activityTools } from './ActivityTimeline';
import { WorkSummary } from './WorkSummary';
import { CommandRunCard } from './CommandRunCard';
import { Chat } from './Chat';
import { toolKindOf } from './TraceIcon';
import type { Message, SessionEvent } from '../lib/types';

const getArtifact = vi.hoisted(() => vi.fn());
vi.mock('../lib/api', () => ({ nativeAvailable: true, api: { getArtifact }, errorMessage: String }));

function event(seq: number, eventType: string, payload: unknown, toolCallId?: string): SessionEvent {
  return { id: `e${seq}`, conversationId: 'audit', runId: 'r1', stepId: 's1', seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false };
}
function tool(id: string, name: string, args: object, result: object): Message {
  return { id, conversationId: 'audit', role: 'tool', status: 'complete', createdAt: 2, reasoning: '', content: JSON.stringify({ request: { name, arguments: args, decision: 'allowed' }, result }) };
}

describe('response audit regressions', () => {
  it('keeps a saved message-only tool beside journal calls', () => {
    const saved = tool('old-write', 'file_write', { path: 'saved.txt' }, { ok: true });
    const result = activityTools([event(3, 'tool_call', { name: 'workspace_read_file' }, 'new-read')], [saved]);
    expect(result).toHaveLength(2);
    expect(result.some(row => row.id === 'old-write')).toBe(true);
  });

  it('classifies native writes and AREX research consistently', () => {
    expect(toolKindOf('file_write', { path: 'result.md' })).toBe('edit');
    expect(toolKindOf('search', { query: ['current releases'] })).toBe('web');
    expect(toolKindOf('visit', { url: ['https://example.com'], goal: 'read' })).toBe('web');
  });

  it('keeps consequential writes out of exploration folding', () => {
    render(<ActivityTimeline events={[]} messages={[
      tool('a', 'file_write', { path: 'a.md' }, { ok: true }),
      tool('b', 'file_write', { path: 'b.md' }, { ok: true }),
    ]} renderTool={m => <span>{m.id}</span>} renderMessage={m => <span>{m.content}</span>} />);
    expect(screen.queryByText('Read 2 files')).not.toBeInTheDocument();
    expect(screen.getByText('a')).toBeInTheDocument();
    expect(screen.getByText('b')).toBeInTheDocument();
  });

  it('reviews batch patch changes and native file_write deliverables', () => {
    render(<WorkSummary messages={[
      tool('patch', 'workspace_apply_patch', { changes: [{ kind: 'create', path: 'created.ts', content: 'hello' }] }, {
        schemaVersion: 'locallm.tool-envelope/1', status: 'success', error: null,
        data: { status: 'committed', changes: [{ path: 'created.ts', kind: 'create', diff: '--- /dev/null\n+++ b/created.ts\n@@ -0,0 +1 @@\n+hello' }] },
      }),
      tool('write', 'file_write', { path: 'report.md', content: 'report' }, { ok: true, path: 'report.md' }),
    ]}><p>Delivered</p></WorkSummary>);
    expect(screen.getByLabelText('Recorded work summary')).toHaveTextContent('2 actions');
    expect(screen.getByLabelText('File changes')).toBeInTheDocument();
    expect(screen.getByText('created.ts')).toBeInTheDocument();
    expect(screen.getByText('report.md')).toBeInTheDocument();
  });

  it('keeps a command opened while running open after successful completion', () => {
    const view = render(<CommandRunCard toolName="shell_run" command="npm test" status="running" stdout="started" />);
    const card = screen.getByTestId('command-run-card');
    expect(card).toHaveAttribute('open');
    view.rerender(<CommandRunCard toolName="shell_run" command="npm test" status="completed" stdout="all tests passed" exitCode={0} />);
    expect(card).toHaveAttribute('open');
  });

  it('hydrates inline citations and the source list from the same artifact', async () => {
    const message = tool('search', 'web_search', {}, { _artifactId: 'art_01234567890123456789012345678901', _truncated: true });
    getArtifact.mockResolvedValue({ conversationId: 'audit', mimeType: 'application/json', content: JSON.stringify({ sources: { s1: { url: 'https://example.com/source', title: 'Recovered evidence' } } }) });
    const answer: Message = { id: 'answer', conversationId: 'audit', role: 'assistant', status: 'complete', content: 'Evidence [s1]', reasoning: '', createdAt: 3 };
    const { container } = render(<Chat messages={[message, answer]} generating={false} ready={false} loading={false} onSend={async () => {}} onCancel={() => {}} onConfigure={() => {}} />);
    await screen.findByRole('link', { name: 'Recovered evidence' });
    await waitFor(() => expect(container.querySelector('.markdown a')).toHaveAttribute('href', 'https://example.com/source'));
  });

  it('folds clipped successful reads without treating clipping as failure', () => {
    const rows = [tool('a', 'workspace_read_file', { path: 'a.md' }, { schemaVersion: 'locallm.tool-envelope/1', status: 'partial', error: null, _truncated: true }),
      tool('b', 'workspace_read_file', { path: 'b.md' }, { schemaVersion: 'locallm.tool-envelope/1', status: 'partial', error: null, _truncated: true })];
    const { container } = render(<ActivityTimeline events={[]} messages={rows} renderTool={m => <p>{m.id}</p>} renderMessage={m => <p>{m.content}</p>} />);
    expect(container.querySelector('.activity-summary')).not.toBeNull();
  });

  it('shows a live process as running with decoded terminal output', () => {
    const message = tool('launch', 'process_start', { command: 'sleep 20' }, {
      schemaVersion: 'locallm.tool-envelope/1', status: 'success', error: null,
      data: { id: 'process-1', state: 'running', exitCode: null, output: [{ cursor: 0, stream: 'stdout', text: 'starting' }], nextCursor: 1 },
    });
    render(<Chat messages={[message]} generating={false} ready={false} loading={false}
      onSend={async () => {}} onCancel={() => {}} onConfigure={() => {}} />);
    expect(screen.queryByTestId('status-finished')).not.toBeInTheDocument();
    expect(screen.getByTestId('status-running')).toBeInTheDocument();
    expect(screen.getByText('starting')).toBeInTheDocument();
  });

  it('makes an opened completed command retain its inspection state', () => {
    // Control: command inspection itself works and should survive the redesign.
    render(<CommandRunCard toolName="shell_run" command="npm test" status="completed" stdout="done" exitCode={0} />);
    const card = screen.getByTestId('command-run-card');
    expect(card).not.toHaveAttribute('open');
    fireEvent.click(card.querySelector('summary')!);
    expect(card).toHaveAttribute('open');
  });
});
