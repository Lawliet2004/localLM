// Diagnostic assertions: passing means the documented problem reproduced.
// These read production components without changing the product or its tests.
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ActivityTimeline, activityTools } from '../../../src/components/ActivityTimeline';
import { WorkSummary } from '../../../src/components/WorkSummary';
import { CommandRunCard } from '../../../src/components/CommandRunCard';
import { AnswerSources, collectAnswerSources } from '../../../src/components/AnswerSources';
import { MarkdownView } from '../../../src/components/MarkdownView';
import { Chat } from '../../../src/components/Chat';
import { toolKindOf } from '../../../src/components/TraceIcon';
import type { Message, SessionEvent } from '../../../src/lib/types';

const getArtifact = vi.hoisted(() => vi.fn());
vi.mock('../../../src/lib/api', () => ({ nativeAvailable: true, api: { getArtifact }, errorMessage: String }));

function event(seq: number, eventType: string, payload: unknown, toolCallId?: string): SessionEvent {
  return { id: `e${seq}`, conversationId: 'audit', runId: 'r1', stepId: 's1', seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false };
}
function tool(id: string, name: string, args: object, result: object): Message {
  return { id, conversationId: 'audit', role: 'tool', status: 'complete', createdAt: 2, reasoning: '', content: JSON.stringify({ request: { name, arguments: args, decision: 'allowed' }, result }) };
}

describe('diagnostic frontend reproductions', () => {
  it('drops a saved message-only tool when any other journal call exists', () => {
    const saved = tool('old-write', 'file_write', { path: 'saved.txt' }, { ok: true });
    const result = activityTools([event(3, 'tool_call', { name: 'workspace_read_file' }, 'new-read')], [saved]);
    expect(result).toHaveLength(1);
    expect(result.some(row => row.id === 'old-write')).toBe(false);
    console.log('CONFIRMED: mixed journal/message-only tool history omits the saved write');
  });

  it('classifies native file writes as reads and AREX web tools as file search/generic tools', () => {
    expect(toolKindOf('file_write', { path: 'result.md' })).toBe('read');
    expect(toolKindOf('search', { query: ['current releases'] })).toBe('search');
    expect(toolKindOf('visit', { url: ['https://example.com'], goal: 'read' })).toBe('tool');
    console.log('CONFIRMED: file_write -> read; AREX search -> file search; visit -> generic tool');
  });

  it('folds successful file writes into a read-files group', () => {
    render(<ActivityTimeline events={[]} messages={[
      tool('a', 'file_write', { path: 'a.md' }, { ok: true }),
      tool('b', 'file_write', { path: 'b.md' }, { ok: true }),
    ]} renderTool={m => <span>{m.id}</span>} renderMessage={m => <span>{m.content}</span>} />);
    expect(screen.getByText('Read 2 files')).toBeInTheDocument();
    console.log('CONFIRMED: two successful writes are hidden behind a Read 2 files disclosure');
  });

  it('omits batch patch changes and basic file_write deliverables from file summary', () => {
    render(<WorkSummary messages={[
      tool('patch', 'workspace_apply_patch', { changes: [{ kind: 'create', path: 'created.ts', content: 'hello' }] }, {
        schemaVersion: 'locallm.tool-envelope/1', status: 'success', error: null,
        data: { status: 'committed', changes: [{ path: 'created.ts', kind: 'create', diff: '--- /dev/null\n+++ b/created.ts\n@@ -0,0 +1 @@\n+hello' }] },
      }),
      tool('write', 'file_write', { path: 'report.md', content: 'report' }, { ok: true, path: 'report.md' }),
    ]}><p>Delivered</p></WorkSummary>);
    expect(screen.getByLabelText('Recorded work summary')).toHaveTextContent('2 actions');
    expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
    console.log('CONFIRMED: successful batch patch and basic file_write have no changed-file summary');
  });

  it('mounts command details live but leaves them expanded after successful completion', () => {
    const view = render(<CommandRunCard toolName="shell_run" command="npm test" status="running" stdout="started" />);
    const card = screen.getByTestId('command-run-card');
    expect(card).toHaveAttribute('open');
    view.rerender(<CommandRunCard toolName="shell_run" command="npm test" status="completed" stdout="all tests passed" exitCode={0} />);
    expect(card).toHaveAttribute('open');
    console.log('CONFIRMED: automatic live expansion persists on success; completed history defaults differently');
  });

  it('hydrates source footer without hydrating the inline citation map', async () => {
    const message = tool('search', 'web_search', {}, { _artifactId: 'art_01234567890123456789012345678901', _truncated: true });
    const initialSources = collectAnswerSources([message], []);
    getArtifact.mockResolvedValue({ conversationId: 'audit', mimeType: 'application/json', content: JSON.stringify({ sources: { s1: { url: 'https://example.com/source', title: 'Recovered evidence' } } }) });
    const { container } = render(<><MarkdownView content="Evidence [s1]" sourceLinks={JSON.stringify(Object.fromEntries(initialSources.flatMap(s => s.ids.map(id => [id, s.url]))))} /><AnswerSources messages={[message]} /></>);
    await waitFor(() => expect(screen.getByRole('link', { name: 'Recovered evidence' })).toBeInTheDocument());
    expect(container.querySelector('.markdown a')).toBeNull();
    expect(screen.getByText('Evidence [s1]')).toBeInTheDocument();
    console.log('CONFIRMED: restored artifact source is linked in footer while the answer citation remains unresolved');
  });

  it('keeps bounded successful reads outside exploration folding because partial means attention', () => {
    const rows = [tool('a', 'workspace_read_file', { path: 'a.md' }, { schemaVersion: 'locallm.tool-envelope/1', status: 'partial', error: null, _truncated: true }),
      tool('b', 'workspace_read_file', { path: 'b.md' }, { schemaVersion: 'locallm.tool-envelope/1', status: 'partial', error: null, _truncated: true })];
    const { container } = render(<ActivityTimeline events={[]} messages={rows} renderTool={m => <p>{m.id}</p>} renderMessage={m => <p>{m.content}</p>} />);
    expect(container.querySelector('.activity-summary')).toBeNull();
    console.log('CONFIRMED: clipped successful reads receive attention grouping even with no operation error');
  });

  it('labels a successful process launch as finished while the process state remains running', () => {
    const message = tool('launch', 'process_start', { command: 'sleep 20' }, {
      schemaVersion: 'locallm.tool-envelope/1', status: 'success', error: null,
      data: { id: 'process-1', state: 'running', exitCode: null, output: [{ cursor: 0, stream: 'stdout', text: 'starting' }], nextCursor: 1 },
    });
    render(<Chat messages={[message]} generating={false} ready={false} loading={false}
      onSend={async () => {}} onCancel={() => {}} onConfigure={() => {}} />);
    expect(screen.getByTestId('status-finished')).toBeInTheDocument();
    expect(screen.queryByTestId('status-running')).not.toBeInTheDocument();
    console.log('CONFIRMED: process_start state=running is presented with a Finished badge');
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
