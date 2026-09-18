import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { ActivityTimeline } from './ActivityTimeline';
import type { Message, SessionEvent } from '../lib/types';

it('interleaves commentary, tool results, compaction and final text without replaying aggregate text', () => {
  const event = (seq: number, eventType: string, payload: unknown, toolCallId?: string): SessionEvent => ({ id: String(seq), conversationId: 'chat', runId: 'run', seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false });
  const messages: Message[] = [{ id: 'answer', conversationId: 'chat', role: 'assistant', status: 'complete', content: 'Checking.Done.', reasoning: '', createdAt: 1 }];
  const { container } = render(<ActivityTimeline messages={messages} events={[
    event(1, 'model_response', { content: 'Checking.' }),
    event(2, 'tool_call', { name: 'execute_command', arguments: { command: 'echo test' } }, 'call'),
    event(3, 'tool_result', { name: 'execute_command', decision: 'allowed', result: { stdout: 'test', exitCode: 0 } }, 'call'),
    event(4, 'compaction', { artifact: 'art-one' }),
    event(5, 'model_response', { content: 'Done.', final: true }),
  ]} renderMessage={m => <p>{m.content}</p>} renderTool={m => <p>{JSON.parse(m.content).result.stdout}</p>} />);
  expect(container.textContent).toMatch(/Checking\..*test.*Context compacted.*Done\./s);
  expect(screen.getAllByText('Done.')).toHaveLength(1);
  expect(container.textContent).not.toContain('Checking.Done.');
});

it('groups exploration actions and formats clean trace items matching reference style', () => {
  const event = (seq: number, eventType: string, payload: unknown, toolCallId?: string): SessionEvent => ({
    id: String(seq), conversationId: 'chat', runId: 'run', seq, eventType, payload, toolCallId, createdAt: seq, ignorable: false,
  });

  const messages: Message[] = [{ id: 'answer', conversationId: 'chat', role: 'assistant', status: 'complete', content: 'Here is the result.', reasoning: '', createdAt: 1 }];

  const events: SessionEvent[] = [
    event(1, 'reasoning', { text: 'Analyzing file contents and permissions.' }),
    event(2, 'tool_call', { name: 'view_file', arguments: { path: 'src/components/ToolControls.tsx', StartLine: 1, EndLine: 60 } }, 'call1'),
    event(3, 'tool_result', { name: 'view_file', decision: 'allowed', result: { stdout: 'line 1\nline 2' } }, 'call1'),
    event(4, 'tool_call', { name: 'grep_search', arguments: { Query: 'PermissionSelector' } }, 'call2'),
    event(5, 'tool_result', { name: 'grep_search', decision: 'allowed', result: { totalResults: 3, stdout: 'match 1\nmatch 2\nmatch 3' } }, 'call2'),
    event(6, 'tool_call', { name: 'edit_file', arguments: { path: 'src/components/Chat.tsx' } }, 'call3'),
    event(7, 'tool_result', { name: 'edit_file', decision: 'allowed', result: { diff: '--- a/Chat.tsx\n+++ b/Chat.tsx\n@@ -1,1 +1,2 @@\n-old\n+new\n+another' } }, 'call3'),
    event(8, 'model_response', { content: 'Here is the result.' }),
  ];

  render(
    <ActivityTimeline
      messages={messages}
      events={events}
      renderMessage={m => <p>{m.content}</p>}
      renderTool={m => {
        const data = JSON.parse(m.content);
        const name = data.request.arguments?.path ? data.request.arguments.path.split('/').pop() : data.request.arguments?.Query;
        return <div data-testid={`tool-${m.id}`}>{data.request.name}: {name}</div>;
      }}
    />
  );

  // Group summary
  expect(screen.getByText('Explored 1 file, 1 search')).toBeInTheDocument();
  // Thought pill
  expect(screen.getByText(/Thought for/)).toBeInTheDocument();
  // Tool outputs are rendered inside the trace
  expect(screen.getByTestId('tool-2')).toHaveTextContent('view_file: ToolControls.tsx');
  expect(screen.getByTestId('tool-4')).toHaveTextContent('grep_search: PermissionSelector');
  expect(screen.getByTestId('tool-6')).toHaveTextContent('edit_file: Chat.tsx');
  // Final response
  expect(screen.getByText('Here is the result.')).toBeInTheDocument();
});

it('uses Explored for completed groups and Exploring only for the active streaming trailing group', () => {
  const event = (seq: number, eventType: string, payload: unknown, toolCallId?: string): SessionEvent => ({
    id: String(seq), conversationId: 'chat', runId: 'run', seq, eventType, payload, toolCallId, createdAt: seq * 1000, ignorable: false,
  });

  const messages: Message[] = [
    { id: 'assistant-live', conversationId: 'chat', role: 'assistant', status: 'streaming', content: '', reasoning: '', createdAt: 1 },
  ];

  const events: SessionEvent[] = [
    // Group 1: 2 files explored
    event(1, 'tool_call', { name: 'view_file', arguments: { path: 'file1.ts' } }, 'c1'),
    event(2, 'tool_result', { name: 'view_file', result: {} }, 'c1'),
    event(3, 'tool_call', { name: 'view_file', arguments: { path: 'file2.ts' } }, 'c2'),
    event(4, 'tool_result', { name: 'view_file', result: {} }, 'c2'),
    // Standalone edit
    event(5, 'tool_call', { name: 'edit_file', arguments: { path: 'file1.ts' } }, 'c3'),
    event(6, 'tool_result', { name: 'edit_file', result: { diff: '--- a/file1.ts\n+++ b/file1.ts\n@@ -1 +1 @@\n-old\n+new' } }, 'c3'),
    // Group 2: active in-progress exploration (1 file currently being analyzed)
    event(7, 'tool_call', { name: 'view_file', arguments: { path: 'Chat.tsx', StartLine: 250, EndLine: 285 } }, 'c4'),
  ];

  render(
    <ActivityTimeline
      messages={messages}
      events={events}
      renderMessage={m => <p>{m.content}</p>}
      renderTool={m => <div data-testid={`tool-${m.id}`}>{m.id}</div>}
    />
  );

  // Completed group before edit must be "Explored 2 files", NOT "Exploring 2 files"
  expect(screen.getByText('Explored 2 files')).toBeInTheDocument();
  // Standalone edit is rendered
  expect(screen.getByTestId('tool-5')).toBeInTheDocument();
  // Trailing active group must be "Exploring 1 file"
  expect(screen.getByText('Exploring 1 file')).toBeInTheDocument();
});

it('synthesizes activity groups and edits from messages when events array is empty', () => {
  const user: Message = { id: 'u1', conversationId: 'chat', role: 'user', content: 'Inspect', reasoning: '', status: 'complete', createdAt: 1 };
  const tool1: Message = {
    id: 't1', conversationId: 'chat', role: 'tool', status: 'complete', reasoning: '', createdAt: 2,
    content: JSON.stringify({ request: { name: 'view_file', arguments: { path: 'src/components/ToolControls.tsx' } }, result: {} }),
  };
  const tool2: Message = {
    id: 't2', conversationId: 'chat', role: 'tool', status: 'complete', reasoning: '', createdAt: 3,
    content: JSON.stringify({ request: { name: 'grep_search', arguments: { Query: 'branch' } }, result: {} }),
  };
  const editTool: Message = {
    id: 't3', conversationId: 'chat', role: 'tool', status: 'complete', reasoning: '', createdAt: 4,
    content: JSON.stringify({ request: { name: 'edit_file', arguments: { path: 'src/components/Chat.tsx' } }, result: { diff: '--- a\n+++ b\n@@ -1 +1 @@\n-a\n+b' } }),
  };
  const assistant: Message = {
    id: 'a1', conversationId: 'chat', role: 'assistant', status: 'complete', content: 'Inspection finished.', reasoning: 'Found the definition.', createdAt: 5,
  };

  render(
    <ActivityTimeline
      messages={[user, tool1, tool2, editTool, assistant]}
      events={[]}
      renderMessage={m => <p>{m.content}</p>}
      renderTool={m => <div data-testid={`tool-${m.id}`}>{m.id}</div>}
    />
  );

  // Synthesized group for the 2 exploration tools
  expect(screen.getByText('Explored 1 file, 1 search')).toBeInTheDocument();
  // Thought pill synthesized from assistant reasoning
  expect(screen.getByText(/Thought/)).toBeInTheDocument();
  // Edit is rendered
  expect(screen.getByTestId('tool-t3')).toBeInTheDocument();
  // Assistant response
  expect(screen.getByText('Inspection finished.')).toBeInTheDocument();
});
