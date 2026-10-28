import { render, screen, fireEvent } from '@testing-library/react';
import { expect, it } from 'vitest';
import { ActivityTimeline, activityTools } from './ActivityTimeline';
import type { Message, SessionEvent } from '../lib/types';
const event = (seq: number, eventType: string, payload: unknown, toolCallId?: string, runId = 'run'): SessionEvent => ({ id: String(seq), conversationId: 'chat', runId, seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false });
const assistant = (status: Message['status'] = 'complete'): Message => ({ id: 'answer', conversationId: 'chat', role: 'assistant', status, content: 'Answer first.', reasoning: 'Private thought.', createdAt: 1 });
const props = { renderMessage: (m: Message) => <p>{m.content}</p>, renderTool: (m: Message) => <p>{JSON.parse(m.content).request.name}</p> };
it('puts collapsed reasoning and activity before the final answer', () => {
  const { container } = render(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'tool_call', { name: 'web_search' }, 's'), event(2, 'tool_result', { name: 'web_search', result: {} }, 's')]} />);
  expect(container.textContent).toMatch(/Thought.*web_search.*Answer first/s);
  expect(screen.getByTestId('turn-reasoning')).not.toHaveAttribute('open');
  expect(screen.getByText('web_search').closest('.activity-summary')).toBeNull();
});
it('keeps one reasoning disclosure mounted across streaming and completed replay', () => {
  const view = render(<ActivityTimeline {...props} messages={[assistant('streaming')]} events={[]} />);
  const disclosure = screen.getByTestId('turn-reasoning');
  fireEvent.click(screen.getByText('Thought'));
  view.rerender(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'reasoning', { text: 'Private thought.', durationMs: 1234 })]} />);
  expect(screen.getByTestId('turn-reasoning')).toBe(disclosure);
  expect(screen.getByText('Thought · a few seconds')).toBeInTheDocument();
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

it('renders separately persisted commentary beside a final-only assistant row', () => {
  const message = { ...assistant(), content: 'Fixed it.', reasoning: '' };
  render(<ActivityTimeline {...props} messages={[message]} events={[
    event(1, 'model_response', { content: 'I will inspect it.', final: false }),
    event(2, 'tool_call', { name: 'read_file' }, 'read'),
    event(3, 'tool_result', { result: {} }, 'read'),
    event(4, 'model_response', { content: 'Fixed it.', final: true }),
    event(5, 'answer_committed', { messageId: message.id, content: 'Fixed it.', format: 'final_only' }),
  ]} />);
  expect(screen.getAllByText('I will inspect it.')).toHaveLength(1);
  expect(screen.getAllByText('Fixed it.')).toHaveLength(1);
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
it('mutes intermediate round commentary while the final round keeps answer styling', () => {
  const message = { ...assistant(), content: 'I will inspect it.Confirmed the cause.Fixed it.', reasoning: '' };
  const events = [
    event(1, 'model_response', { content: 'I will inspect it.', final: false }),
    event(2, 'model_response', { content: 'Confirmed the cause.', final: false }),
    event(3, 'model_response', { content: 'Fixed it.', final: true }),
  ];
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={events} />);
  const commentary = container.querySelectorAll('.activity-commentary');
  expect(commentary).toHaveLength(2);
  expect(commentary[0].textContent).toBe('I will inspect it.');
  expect(commentary[1].textContent).toBe('Confirmed the cause.');
  // The final round renders outside the commentary wrapper.
  const final = screen.getByText('Fixed it.');
  expect(final.closest('.activity-commentary')).toBeNull();
});

it('keeps each reasoning round between its surrounding tools and commentary', () => {
  const events = [
    event(1, 'reasoning', { text: 'Inspect first.', durationMs: 1200 }),
    event(2, 'model_response', { content: 'Looking at files.', final: false }),
    event(3, 'tool_call', { name: 'read_file' }, 'read'),
    event(4, 'tool_result', { result: {} }, 'read'),
    event(5, 'reasoning', { text: 'Now verify.', durationMs: 2000 }),
    event(6, 'tool_call', { name: 'execute_command' }, 'cmd'),
    event(7, 'tool_result', { result: {} }, 'cmd'),
  ];
  const message = { ...assistant(), reasoning: 'Inspect first.Now verify.', content: 'Looking at files.Done.' };
  const { container } = render(<ActivityTimeline {...props} messages={[message]} events={[...events].reverse().concat(events[0])} />);
  expect(container.textContent).toMatch(/Inspect first.*Looking at files.*read_file.*Now verify.*execute_command.*Done/s);
  expect(screen.getAllByTestId('turn-reasoning')).toHaveLength(2);
  expect(screen.getAllByText('Thought · a few seconds')).toHaveLength(2);
});

it('keeps a streaming next-round thought in place when its journal arrives', () => {
  const events = [event(1, 'reasoning', { text: 'First.' }), event(2, 'tool_call', { name: 'web_search' }, 'search'), event(3, 'tool_result', { result: {} }, 'search')];
  const message = { ...assistant('streaming'), reasoning: 'First.Next.', content: '' };
  const view = render(<ActivityTimeline {...props} messages={[message]} events={events} generating />);
  const thoughts = screen.getAllByTestId('turn-reasoning');
  expect(view.container.textContent).toMatch(/First.*web_search.*Thinking.*Next/s);
  view.rerender(<ActivityTimeline {...props} messages={[{ ...message, status: 'complete', content: 'Done.' }]} events={[...events, event(4, 'reasoning', { text: 'Next.', durationMs: 500 })]} />);
  expect(screen.getAllByTestId('turn-reasoning')[1]).toBe(thoughts[1]);
  expect(screen.getAllByText('Next.')).toHaveLength(1);
});

it('preserves unmatched reasoning without fabricating per-round timing', () => {
  render(<ActivityTimeline {...props} messages={[assistant()]} events={[event(1, 'reasoning', { text: 'Stale.', durationMs: 500 })]} />);
  expect(screen.getByText('Private thought.')).toBeInTheDocument();
  expect(screen.queryByText('Stale.')).not.toBeInTheDocument();
  expect(screen.queryByText('Thought · a few seconds')).not.toBeInTheDocument();
});

it('summarizes mixed exploration with search and distinct file counts', () => {
  const names = ['search_files', 'read_file', 'read_file'];
  const messages: Message[] = names.map((name, i) => ({ ...assistant(), id: String(i), role: 'tool', content: JSON.stringify({ request: { name, arguments: { path: 'src/app.ts' } }, result: {} }) }));
  render(<ActivityTimeline {...props} messages={messages} events={[]} />);
  expect(screen.getByText('Explore · 1 search, 1 file')).toBeInTheDocument();
});

it('retains tools missing from a partial journal without duplicating matched rows', () => {
  const saved = (id: string): Message => ({ ...assistant(), id: `tool-${id}`, role: 'tool', content: JSON.stringify({ request: { name: 'read_file', callId: id, runId: 'run' }, result: {} }) });
  const first = saved('first'), next = saved('next');
  const events = [event(1, 'tool_call', { name: 'read_file' }, 'first')];
  expect(activityTools(events, [first, next])).toEqual([first, next]);
});

it('preserves inspected tool details when the next reasoning round is committed', () => {
  const events = [event(1, 'reasoning', { text: 'First.' }), event(2, 'tool_call', { name: 'execute_command' }, 'cmd'), event(3, 'tool_result', { result: {} }, 'cmd')];
  const message = { ...assistant(), reasoning: 'First.Next.' };
  const renderTool = () => <details data-testid="inspected-tool"><summary>Command</summary>Output</details>;
  const view = render(<ActivityTimeline {...props} renderTool={renderTool} messages={[message]} events={events} />);
  const detail = screen.getByTestId('inspected-tool');
  detail.setAttribute('open', '');
  view.rerender(<ActivityTimeline {...props} renderTool={renderTool} messages={[message]} events={[...events, event(4, 'reasoning', { text: 'Next.' })]} />);
  expect(screen.getByTestId('inspected-tool')).toBe(detail);
  expect(detail).toHaveAttribute('open');
});

it('keeps unjournaled tools in timestamp order between recorded commentary', () => {
  const message = { ...assistant(), reasoning: '', content: 'Starting.Middle.Done.' };
  const missing: Message = { ...assistant(), id: 'missing', role: 'tool', createdAt: 3, content: JSON.stringify({ request: { name: 'legacy_read' }, result: {} }) };
  const events = [event(1, 'model_response', { content: 'Starting.', final: false }), event(5, 'model_response', { content: 'Middle.', final: false }), event(8, 'tool_call', { name: 'journal_read' }, 'recorded'), event(9, 'tool_result', { result: {} }, 'recorded')];
  const { container } = render(<ActivityTimeline {...props} messages={[message, missing]} events={events} />);
  expect(container.textContent).toMatch(/Starting.*legacy_read.*Middle.*journal_read.*Done/s);
});

it('does not invent a thought or duration when the provider emits no reasoning', () => {
  render(<ActivityTimeline {...props} messages={[{ ...assistant('streaming'), reasoning: '', content: '' }]} events={[]} generating />);
  expect(screen.queryByTestId('turn-reasoning')).not.toBeInTheDocument();
});

it('keeps provider call IDs distinct when reused in different steps of the same run', () => {
  const events = [
    { ...event(1, 'tool_call', { name: 'first' }, 'same'), stepId: 'step-1' },
    { ...event(2, 'tool_result', { result: { value: 'one' } }, 'same'), stepId: 'step-1' },
    { ...event(3, 'tool_call', { name: 'second' }, 'same'), stepId: 'step-2' },
    { ...event(4, 'tool_result', { result: { value: 'two' } }, 'same'), stepId: 'step-2' },
  ];
  const saved = (stepId: string, value: string, createdAt: number): Message => ({ ...assistant(), id: `tool-${stepId}`, createdAt, role: 'tool', content: JSON.stringify({ request: { name: stepId, callId: 'same', runId: 'run', stepId }, result: { value } }) });
  const first = saved('step-1', 'one', 2), second = saved('step-2', 'two', 4);
  expect(activityTools(events, [second, first])).toEqual([first, second]);
  expect(activityTools(events, []).map(message => JSON.parse(message.content).result.value)).toEqual(['one', 'two']);
});

it('places a live next-round thought after prior tools and reconciles it without remounting', async () => {
  const { appendModelDelta } = await import('../lib/agentTranscript');
  let live = { ...assistant('streaming'), content: '', reasoning: '' };
  live = appendModelDelta(live, { messageId: live.id, runId: 'run', stepId: 'one', seq: 2, roundIndex: 0, content: 'Inspect.', reasoning: 'Read.' });
  live = appendModelDelta(live, { messageId: live.id, runId: 'run', stepId: 'two', seq: 20, roundIndex: 1, content: '', reasoning: 'Verify.' });
  const journal = [
    { ...event(1, 'step_start', {}), stepId: 'one' },
    { ...event(2, 'reasoning', { text: 'Read.' }), stepId: 'one' },
    { ...event(3, 'model_response', { content: 'Inspect.', final: false }), stepId: 'one' },
    { ...event(4, 'tool_call', { name: 'read_file' }, 'read'), stepId: 'one' },
    { ...event(5, 'tool_result', { result: {} }, 'read'), stepId: 'one' },
    { ...event(6, 'step_start', {}), stepId: 'two' },
  ];
  const view = render(<ActivityTimeline {...props} messages={[live]} events={journal} generating />);
  expect(view.container.textContent).toMatch(/Read\..*Inspect\..*read_file.*Verify\./s);
  const thought = screen.getAllByTestId('turn-reasoning')[1];
  view.rerender(<ActivityTimeline {...props} messages={[live]} events={[...journal, { ...event(7, 'reasoning', { text: 'Verify.', durationMs: 1000 }), stepId: 'two' }]} generating />);
  expect(screen.getAllByTestId('turn-reasoning')[1]).toBe(thought);
  expect(screen.getAllByText('Verify.')).toHaveLength(1);
});

it('does not fold legacy nested partial outcomes into successful exploration', () => {
  const tools = ['one', 'two'].map(id => ({ ...assistant(), id, role: 'tool' as const, content: JSON.stringify({ request: { name: 'read_file', arguments: { path: id } }, result: { data: { status: 'partial' } } }) }));
  const { container } = render(<ActivityTimeline {...props} events={[]} messages={tools} />);
  expect(container.querySelector('.activity-summary')).toBeNull();
});
