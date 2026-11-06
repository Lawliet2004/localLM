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
it('respects the reasoning disclosure while streaming and after completion', () => {
  const message = { ...assistant('streaming'), content: '', reasoning: 'First thought' };
  const view = render(<ActivityTimeline {...props} messages={[message]} events={[]} generating />);
  const disclosure = screen.getByTestId('turn-reasoning');
  expect(disclosure).not.toHaveAttribute('open');
  fireEvent.click(screen.getByLabelText('Reasoning in progress'));
  expect(disclosure).toHaveAttribute('open');
  view.rerender(<ActivityTimeline {...props} messages={[{ ...message, reasoning: 'First thought and more' }]} events={[]} generating />);
  expect(disclosure).toHaveAttribute('open');
  view.rerender(<ActivityTimeline {...props} messages={[{ ...message, status: 'complete', content: 'Answer' }]} events={[]} />);
  expect(disclosure).toHaveAttribute('open');
});
it('preserves reasoning from each assistant message without duplicating journal replay', () => {
  const first = { ...assistant(), id: 'first', reasoning: 'First investigation' };
  const second = { ...assistant(), id: 'second', reasoning: 'Second investigation' };
  render(<ActivityTimeline {...props} messages={[first, second]} events={[event(1, 'reasoning', { text: 'First investigation' })]} />);
  fireEvent.click(screen.getByText('Reasoning'));
  expect(screen.getByText('First investigation')).toBeInTheDocument();
  expect(screen.getByText('Second investigation')).toBeInTheDocument();
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

it('uses live and replayed approval items for only the matching tool call', () => {
  const call = { ...event(1, 'tool_call', { name: 'workspace_edit_file', arguments: { path: 'app.ts' } }, 'same'), stepId: 'step-a' };
  const other = { ...event(2, 'tool_call', { name: 'workspace_read_file', arguments: { path: 'other.ts' } }, 'same'), stepId: 'step-b' };
  const pending = { ...event(3, 'item.started', { item: { id: 'approval', runId: 'run', round: 0, kind: 'approval', status: 'awaiting', startedAt: 3, payload: { callId: 'same', stepId: 'step-a', name: 'workspace_edit_file' } } }), stepId: 'step-a' };
  const update = event(4, 'item.updated', { itemId: 'approval', payload: { decision: 'denied' } });
  const complete = event(5, 'item.completed', { itemId: 'approval', status: 'completed', endedAt: 5 });
  const result = { ...event(6, 'tool_result', { name: 'workspace_edit_file', decision: 'denied', result: { isError: true } }, 'same'), stepId: 'step-a' };
  const live = activityTools([call, other, pending], []);
  expect(live).toHaveLength(2);
  expect(JSON.parse(live[0].content).request.decision).toBe('pending');
  expect(JSON.parse(live[1].content).request.decision).toBeUndefined();
  const replayed = activityTools([result, complete, update, pending, other, call], []);
  expect(JSON.parse(replayed[0].content).request.decision).toBe('denied');
  expect(replayed[0].status).toBe('complete');
  expect(replayed[1].status).toBe('streaming');
});

it('does not keep a stale approval pending after the turn ended', () => {
  const call = event(1, 'tool_call', { name: 'workspace_edit_file' }, 'edit');
  const pending = event(2, 'item.started', { item: { id: 'approval', runId: 'run', round: 0, kind: 'approval', status: 'awaiting', startedAt: 2, payload: { callId: 'edit', stepId: null } } });
  const ended = event(3, 'turn_end', {});
  const [tool] = activityTools([call, pending, ended], []);
  expect(tool.status).toBe('interrupted');
  expect(JSON.parse(tool.content).request.decision).toBeUndefined();
  const [orphaned] = activityTools([call, pending], [], undefined, false);
  expect(orphaned.status).toBe('interrupted');
  const cancelled = event(3, 'item.completed', { itemId: 'approval', status: 'interrupted', endedAt: 3 });
  const [cancelledTool] = activityTools([call, pending, cancelled], []);
  expect(cancelledTool.status).toBe('interrupted');
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

it('interleaves recorded reasoning rounds with the actions they precede', () => {
  const message = { ...assistant(), content: 'Inspecting.Fixed.', reasoning: 'First thought.Second thought.' };
  const events = [
    event(1, 'reasoning', { text: 'First thought.', durationMs: 1200 }),
    event(2, 'model_response', { content: 'Inspecting.', final: false }),
    event(3, 'tool_call', { name: 'read_file', arguments: { path: 'app.ts' } }, 'read'),
    event(4, 'tool_result', { name: 'read_file', result: {} }, 'read'),
    event(5, 'reasoning', { text: 'Second thought.', durationMs: 2300 }),
    event(6, 'model_response', { content: 'Fixed.', final: true }),
  ];
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={events} />);
  expect(container.textContent).toMatch(/Reasoning · 1.2s.*First thought.*Inspecting.*read_file.*Reasoning · 2.3s.*Second thought.*Fixed/s);
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(2);
});

it('renders typed live reasoning in order and uses the same result on replay', () => {
  const message = { ...assistant('streaming'), content: 'Inspecting.', reasoning: 'First thought.' };
  const started = event(1, 'item.started', { item: { id: 'thought', runId: 'run', round: 0, kind: 'reasoning', status: 'running', startedAt: 10, payload: {} } });
  const delta = event(2, 'item.delta', { itemId: 'thought', delta: { text: 'First thought.' } });
  const done = event(3, 'item.completed', { itemId: 'thought', status: 'completed', endedAt: 1210, durationMs: 1200 });
  const tool = event(4, 'tool_call', { name: 'read_file' }, 'read');
  const view = render(<ActivityTimeline {...props} messages={[message]} events={[started, delta]} generating />);
  const disclosure = screen.getByTestId('turn-reasoning');
  expect(screen.getByLabelText('Reasoning in progress')).toBeInTheDocument();
  view.rerender(<ActivityTimeline {...props} messages={[{ ...message, status: 'complete' }]} events={[tool, done, delta, started, delta]} />);
  expect(screen.getByTestId('turn-reasoning')).toBe(disclosure);
  expect(view.container.textContent).toMatch(/Reasoning · 1.2s.*First thought.*read_file.*Inspecting/s);
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(1);
});

it('uses complete legacy reasoning when a typed journal item never finished', () => {
  const message = { ...assistant(), reasoning: 'A complete thought.' };
  const started = event(1, 'item.started', { item: { id: 'thought', runId: 'run', round: 0, kind: 'reasoning', status: 'running', startedAt: 10, payload: {} } });
  const delta = event(2, 'item.delta', { itemId: 'thought', delta: { text: 'A partial' } });
  const legacy = event(3, 'reasoning', { text: 'A complete thought.', durationMs: 400 });
  render(<ActivityTimeline {...props} messages={[message]} events={[started, delta, legacy]} />);
  expect(screen.getByText('Reasoning · 0.4s')).toBeInTheDocument();
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(1);
  expect(screen.queryByText('A partial')).not.toBeInTheDocument();
});

it('shows a tool request while arguments are being written, then hands off to the recorded call', () => {
  const message = { ...assistant('streaming'), content: '', reasoning: '' };
  const start = event(1, 'item.started', { item: { id: 'draft', runId: 'run', round: 0, kind: 'tool', status: 'running', startedAt: 1, payload: { phase: 'preparing', name: 'workspace_edit_file', arguments: {} } } });
  const update = event(2, 'item.updated', { itemId: 'draft', payload: { phase: 'preparing', name: 'workspace_edit_file', callId: 'call', arguments: { path: 'src/App.tsx' } } });
  const end = event(3, 'item.completed', { itemId: 'draft', status: 'completed', endedAt: 4 });
  const call = event(4, 'tool_call', { name: 'edit_file', arguments: { path: 'src/App.tsx' } }, 'call');
  const view = render(<ActivityTimeline {...props} messages={[message]} events={[start]} generating />);
  expect(screen.getByText('Preparing edit request')).toBeInTheDocument();
  view.rerender(<ActivityTimeline {...props} messages={[message]} events={[start, update]} generating />);
  expect(screen.getByText('src/App.tsx')).toBeInTheDocument();
  view.rerender(<ActivityTimeline {...props} messages={[message]} events={[start, update, end, call]} generating />);
  expect(screen.queryByText('Preparing edit request')).not.toBeInTheDocument();
  expect(screen.getByText('edit_file')).toBeInTheDocument();
});

it('does not imply an incomplete tool request executed', () => {
  const start = event(1, 'item.started', { item: { id: 'draft', runId: 'run', round: 0, kind: 'tool', status: 'running', startedAt: 1, payload: { name: 'workspace_edit_file', arguments: { path: 'app.ts' } } } });
  const end = event(2, 'item.completed', { itemId: 'draft', status: 'interrupted', endedAt: 3 });
  render(<ActivityTimeline {...props} messages={[{ ...assistant(), reasoning: '' }]} events={[start, end]} />);
  expect(screen.getByText('Tool request incomplete · not run')).toBeInTheDocument();
});

it('keeps an interrupted draft before a later tool call even without reasoning', () => {
  const message = { ...assistant(), reasoning: '' };
  const start = event(1, 'item.started', { item: { id: 'draft', runId: 'run', round: 0, kind: 'tool', status: 'running', startedAt: 1, payload: { name: 'workspace_edit_file', arguments: { path: 'app.ts' } } } });
  const end = event(2, 'item.completed', { itemId: 'draft', status: 'interrupted', endedAt: 3 });
  const call = event(3, 'tool_call', { name: 'read_file' }, 'read');
  const result = event(4, 'tool_result', { name: 'read_file', result: {} }, 'read');
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={[start, end, call, result]} />);
  expect(container.textContent).toMatch(/Tool request incomplete · not run.*read_file.*Answer first/s);
});

it('does not group exploration across an unfinished tool request', () => {
  const message = { ...assistant('streaming'), reasoning: '' };
  const draft = event(3, 'item.started', { item: { id: 'draft', runId: 'run', round: 1, kind: 'tool', status: 'running', startedAt: 3, payload: { name: 'workspace_edit_file', arguments: { path: 'app.ts' } } } });
  const events = [event(1, 'tool_call', { name: 'read_first', arguments: { path: 'a.ts' } }, 'first'),
    event(2, 'tool_result', { name: 'read_first', result: {} }, 'first'), draft,
    event(4, 'tool_call', { name: 'read_second', arguments: { path: 'b.ts' } }, 'second'),
    event(5, 'tool_result', { name: 'read_second', result: {} }, 'second')];
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={events} generating />);
  expect(container.textContent).toMatch(/read_first.*Preparing edit request.*read_second/s);
  expect(container.querySelector('.activity-summary')).toBeNull();
});

it('preserves unrecorded live thought and falls back when journal text conflicts', () => {
  const message = { ...assistant('streaming'), content: 'Inspecting.', reasoning: 'First thought.Second thought.' };
  const events = [event(1, 'reasoning', { text: 'First thought.' }), event(2, 'tool_call', { name: 'read_file' }, 'read')];
  const view = render(<ActivityTimeline {...props} messages={[message]} events={events} generating />);
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(2);
  expect(screen.getByLabelText('Reasoning in progress')).toBeInTheDocument();
  view.rerender(<ActivityTimeline {...props} messages={[message]} events={[event(1, 'reasoning', { text: 'Different thought.' })]} generating />);
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(1);
  fireEvent.click(screen.getByText('Reasoning'));
  expect(screen.getByText('First thought.Second thought.')).toBeInTheDocument();
  expect(screen.queryByText('Different thought.')).not.toBeInTheDocument();
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
