import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import { ActivityTimeline } from './ActivityTimeline';
import { answerSourceLinks } from './AnswerSources';
import type { Message } from '../lib/types';
const mocks = vi.hoisted(() => ({ getOwnedProcess: vi.fn(), stopOwnedProcess: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: true, errorMessage: String, api: mocks }));
const snapshot = (state: string) => ({ id: 'owned-p', state, exitCode: null, output: [{ cursor: 0, stream: 'stdout', text: 'process output' }], nextCursor: 1, hasMore: false });
const message = (id: string, name: string, state: string, cursor = 0): Message => ({ id, conversationId: 'c', role: 'tool', reasoning: '', status: 'complete', createdAt: cursor,
  content: JSON.stringify({ request: { connector: 'Harness', name, arguments: { command: 'sleep 20' } }, result: { schemaVersion: 'locallm.tool-envelope/1', status: 'success', error: null, metadata: { family: 'harness' }, data: { ...snapshot(state), output: [{ cursor, stream: 'stdout', text: `page ${cursor}` }] } } }) });
beforeEach(() => vi.clearAllMocks());
it('uses the owned process stop control and updates its terminal state', async () => {
  mocks.getOwnedProcess.mockResolvedValue(snapshot('running'));
  mocks.stopOwnedProcess.mockResolvedValue(snapshot('stopped'));
  const cancel = vi.fn();
  render(<Chat messages={[message('start', 'process_start', 'running')]} generating={false} ready={false} loading={false} onSend={async () => {}} onCancel={cancel} onConfigure={() => {}} />);
  await screen.findByText('process output');
  expect(mocks.getOwnedProcess).toHaveBeenCalledWith('c', 'owned-p', 0);
  mocks.getOwnedProcess.mockResolvedValue(snapshot('stopped'));
  fireEvent.click(screen.getByRole('button', { name: 'Stop process' }));
  await waitFor(() => expect(mocks.stopOwnedProcess).toHaveBeenCalledWith('c', 'owned-p'));
  await waitFor(() => expect(screen.getByTestId('status-interrupted')).toHaveTextContent('Stopped'));
  expect(cancel).not.toHaveBeenCalled();
});
it('coalesces saved poll pages into the launched process row', () => {
  render(<ActivityTimeline events={[]} messages={[message('start', 'process_start', 'running'), message('poll', 'process_poll', 'exited', 1)]}
    renderMessage={m => <p>{m.content}</p>} renderTool={m => { const record = JSON.parse(m.content); return <p>{record.request.name} {record.result.data.state} {record.result.data.output.map((frame: { text: string }) => frame.text).join(' ')}</p>; }} />);
  expect(screen.getByText('process_start exited page 0 page 1')).toBeInTheDocument();
  expect(screen.queryByText(/process_poll/)).not.toBeInTheDocument();
});
it('does not choose an arbitrary URL for an ambiguous citation ID', () => {
  expect(JSON.parse(answerSourceLinks([{ url: 'https://one.example/', ids: ['1', 'a'] }, { url: 'https://two.example/', ids: ['1', 'b'] }]))).toEqual({ a: 'https://one.example/', b: 'https://two.example/' });
});
