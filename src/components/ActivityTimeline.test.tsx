import { render, screen, fireEvent } from '@testing-library/react';
import { expect, it } from 'vitest';
import { ActivityTimeline, activityTools } from './ActivityTimeline';
import type { Message, SessionEvent } from '../lib/types';
const event = (seq: number, eventType: string, payload: unknown, toolCallId?: string, runId = 'run'): SessionEvent => ({ id: String(seq), conversationId: 'chat', runId, seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false });
const assistant = (status: Message['status'] = 'complete'): Message => ({ id: 'answer', conversationId: 'chat', role: 'assistant', status, content: 'Answer first.', reasoning: 'Private thought.', createdAt: 1 });
const props = { renderMessage: (m: Message) => <p>{m.content}</p>, renderTool: (m: Message) => <p>{JSON.parse(m.content).request.name}</p> };
it('puts collapsed reasoning and activity before the final answer', () => {
  const { container } = render(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'tool_call', { name: 'web_search' }, 's'), event(2, 'tool_result', { name: 'web_search', result: {} }, 's')]} />);
  expect(container.textContent).toMatch(/Reasoning.*web_search.*Answer first/s);
  expect(screen.getByTestId('turn-reasoning')).not.toHaveAttribute('open');
  expect(screen.getByText('web_search').closest('.activity-summary')).toBeNull();
});
it('keeps one reasoning disclosure mounted across streaming and completed replay', () => {
  const view = render(<ActivityTimeline {...props} messages={[assistant('streaming')]} events={[]} />);
  const disclosure = screen.getByTestId('turn-reasoning');
  fireEvent.click(screen.getByText('Reasoning'));
  view.rerender(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'reasoning', { text: 'Private thought.', durationMs: 1234 })]} />);
  expect(screen.getByTestId('turn-reasoning')).toBe(disclosure);
  expect(screen.getByText('Reasoning · 1.2s')).toBeInTheDocument();
});
it('does not fabricate durations or replay per-round text', () => {
  const { container } = render(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'reasoning', { text: 'Thought' }), event(2, 'model_response', { content: 'Earlier commentary' })]} />);
  expect(container.textContent).not.toMatch(/8s|Thought for|Earlier commentary/);
  expect(screen.getAllByText('Answer first.')).toHaveLength(1);
});
it('keeps pending approvals and errors outside collapsed activity', () => {
  render(<ActivityTimeline {...props} messages={[assistant('streaming')]} events={[event(1, 'tool_call', { name: 'execute_command' }, 'c')]} />);
  expect(screen.getByText('execute_command').closest('details')).toBeNull();
});
it('retains message-only history and filters plan updates', () => {
  const tool = (name: string): Message => ({ ...assistant(), id: name, role: 'tool', content: JSON.stringify({ request: { name }, result: {} }) });
  const result = activityTools([], [tool('todo_write'), tool('view_file')]);
  expect(result.map(m => m.id)).toEqual(['view_file']);
});
it('joins by run and tool id without crossing repeated tool ids', () => {
  const tools = activityTools([event(1, 'tool_call', { name: 'one' }, 'same'), event(2, 'tool_call', { name: 'two' }, 'same', 'other'), event(3, 'tool_result', { name: 'two', result: { value: 2 } }, 'same', 'other')], []);
  expect(tools[0].status).toBe('streaming');
  expect(JSON.parse(tools[1].content).result.value).toBe(2);
});

it('interleaves recorded commentary and tool groups without repeating the cumulative answer', () => {
  const message = { ...assistant(), content: 'I will inspect it.Confirmed the cause.Fixed it.', reasoning: '' };
  const events = [
    event(1, 'model_response', { content: 'I will inspect it.', final: false }),
    event(2, 'tool_call', { name: 'read_file', arguments: { path: 'app.ts' } }, 'read'),
    event(3, 'tool_result', { name: 'read_file', result: {} }, 'read'),
    event(4, 'model_response', { content: 'Confirmed the cause.', final: false }),
    event(5, 'tool_call', { name: 'edit_file', arguments: { path: 'app.ts' } }, 'edit'),
    event(6, 'tool_result', { name: 'edit_file', result: {} }, 'edit'),
    event(7, 'model_response', { content: 'Fixed it.', final: true }),
  ];
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={events} />);
  expect(container.textContent).toMatch(/I will inspect it.*read_file.*Confirmed the cause.*edit_file.*Fixed it/s);
  expect(screen.getAllByText('I will inspect it.')).toHaveLength(1);
  expect(screen.getAllByText('Confirmed the cause.')).toHaveLength(1);
  expect(screen.getAllByText('Fixed it.')).toHaveLength(1);
});

it('preserves the complete answer when journal commentary does not match', () => {
  render(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'model_response', { content: 'Stale commentary', final: false })]} />);
  expect(screen.getAllByText('Answer first.')).toHaveLength(1);
  expect(screen.queryByText('Stale commentary')).not.toBeInTheDocument();
});

it('keeps envelope failures visible outside the collapsed group', () => {
  const tool: Message = { ...assistant(), role: 'tool', content: JSON.stringify({ request: { name: 'execute_command' }, result: { status: 'failure' } }) };
  render(<ActivityTimeline {...props} messages={[tool]} events={[]} />);
  expect(screen.getByText('execute_command').closest('details')).toBeNull();
});

it('folds only adjacent successful exploration and preserves errors and edits in place', () => {
  const tool = (id: string, name: string, status: Message['status'] = 'complete', decision = 'allowed'): Message => ({ ...assistant(status), id, role: 'tool', content: JSON.stringify({ request: { name, arguments: { path: `${id}.ts` }, decision }, result: {} }) });
  const tools = [tool('r1', 'read_file'), tool('r2', 'read_file'), tool('bad', 'search', 'error'), tool('r3', 'read_file'), tool('edit', 'edit_file'), tool('pending', 'execute_command', 'streaming', 'pending'), tool('r4', 'read_file'), tool('r5', 'read_file')];
  const { container } = render(<ActivityTimeline events={[]} messages={tools} {...props} renderTool={m => <p data-tool={m.id}>{m.id}</p>} />);
  expect([...container.querySelectorAll('[data-tool]')].map(row => row.getAttribute('data-tool'))).toEqual(tools.map(tool => tool.id));
  expect(container.querySelectorAll('.activity-summary')).toHaveLength(2);
  for (const id of ['bad', 'edit', 'pending']) expect(container.querySelector(`[data-tool="${id}"]`)?.closest('.activity-summary')).toBeNull();
  expect(screen.getAllByText('Read 2 files')).toHaveLength(2);
});

it('deduplicates journal deliveries and orders calls by sequence', () => {
  const first = event(1, 'tool_call', { name: 'read_file' }, 'first');
  const second = event(2, 'tool_call', { name: 'edit_file' }, 'second');
  expect(activityTools([second, first, first, event(3, 'tool_result', { result: {} }, 'first')], []).map(tool => JSON.parse(tool.content).request.name)).toEqual(['read_file', 'edit_file']);
});

it('does not turn a recorded tool call into a fabricated approval', () => {
  const [tool] = activityTools([event(1, 'tool_call', { name: 'read_file' }, 'call')], []);
  expect(tool.status).toBe('streaming');
  expect(JSON.parse(tool.content).request.decision).toBeUndefined();
  expect(tool.conversationId).toBe('chat');
});

it('keeps replay row identity and never attaches a later result to a reused call ID', () => {
  const events = [event(1, 'tool_call', { name: 'read_file' }, 'same', 'one'), event(2, 'tool_result', { result: { value: 'old' } }, 'same', 'one'), event(3, 'tool_call', { name: 'read_file' }, 'same', 'two'), event(4, 'tool_result', { result: { value: 'new' } }, 'same', 'two')];
  const persisted: Message = { ...assistant(), id: 'tool-same', role: 'tool', createdAt: 4, content: JSON.stringify({ request: { name: 'read_file', callId: 'same', runId: 'two' }, result: { value: 'new' } }) };
  const first = activityTools(events, [persisted]);
  const second = activityTools(events, [persisted, assistant('streaming')]);
  expect(JSON.parse(first[0].content).result.value).toBe('old');
  expect(first[1]).toBe(persisted); expect(second[0]).toBe(first[0]);
});
